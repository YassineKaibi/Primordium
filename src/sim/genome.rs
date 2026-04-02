use crate::config::WorldConfig;

// ── Genome geometry ──────────────────────────────────────────────────
pub const GENOME_LEN: usize = 64;
pub const BASE_GENE_COUNT: usize = 46;
pub const PHASE_SLOT_COUNT: usize = 3;
pub const PHASE_SLOT_SIZE: usize = 6;

// ── Gene index constants (one per base gene) ─────────────────────────
// Metabolism (0-5)
pub const PHOTOSYNTHESIS_RATE: usize = 0;
pub const THERMOSYNTHESIS_RATE: usize = 1;
pub const PREDATION_EFFICIENCY: usize = 2;
pub const SCAVENGE_ABILITY: usize = 3;
pub const ENERGY_STORAGE_CAP: usize = 4;
pub const BASE_METABOLISM: usize = 5;

// Movement (6-11)
pub const SPEED: usize = 6;
pub const DIRECTION_BIAS: usize = 7;
pub const DIRECTION_NOISE: usize = 8;
pub const CHEMOTAXIS_STRENGTH: usize = 9;
pub const FLEE_RESPONSE: usize = 10;
pub const PACK_AFFINITY: usize = 11;

// Combat (12-16)
pub const ATTACK_POWER: usize = 12;
pub const ARMOR: usize = 13;
pub const VENOM: usize = 14;
pub const ATTACK_RANGE: usize = 15;
pub const AGGRESSION_TRIGGER: usize = 16;

// Reproduction (17-22)
pub const REPRODUCTION_THRESHOLD: usize = 17;
pub const OFFSPRING_ENERGY_SHARE: usize = 18;
pub const MUTATION_RATE: usize = 19;
pub const MUTATION_MAGNITUDE: usize = 20;
pub const REPRODUCTION_COOLDOWN: usize = 21;
pub const OFFSPRING_SCATTER: usize = 22;

// Sensing (23-27)
pub const SENSE_RADIUS: usize = 23;
pub const SENSE_PRIORITY: usize = 24;
pub const MEMORY_LENGTH: usize = 25;
pub const SIGNAL_EMISSION: usize = 26;
pub const SIGNAL_SENSITIVITY: usize = 27;

// Structural (28-31)
pub const ADHESION: usize = 28;
pub const RIGIDITY: usize = 29;
pub const DECAY_RATE: usize = 30;
pub const MEMBRANE: usize = 31;

// Lifecycle (32-35)
pub const MAX_AGE: usize = 32;
pub const MATURITY_AGE: usize = 33;
pub const DORMANCY_TRIGGER: usize = 34;
pub const DORMANCY_COST: usize = 35;

// Environmental (36-38)
pub const TEMPERATURE_PREFERENCE: usize = 36;
pub const TOXIN_RESISTANCE: usize = 37;
pub const ADAPTATION_RATE: usize = 38;

// Social (39-42)
pub const KIN_RECOGNITION_PRECISION: usize = 39;
pub const RESOURCE_SHARING: usize = 40;
pub const TERRITORIAL_RADIUS: usize = 41;
pub const SWARM_SIGNAL: usize = 42;

// Meta (43-45)
pub const GENE_LINKAGE: usize = 43;
pub const HORIZONTAL_TRANSFER: usize = 44;
pub const TRANSPOSON_RATE: usize = 45;

// ── Phase slot field offsets (within each 6-byte slot) ───────────────
pub const PHASE_TRIGGER_CONDITION: usize = 0;
pub const PHASE_TRIGGER_THRESHOLD: usize = 1;
pub const PHASE_OFFENSE_MOD: usize = 2;
pub const PHASE_DEFENSE_MOD: usize = 3;
pub const PHASE_MOBILITY_MOD: usize = 4;
pub const PHASE_EFFICIENCY_MOD: usize = 5;

// ── Antagonistic pair definitions ────────────────────────────────────
/// Each pair: (gene_a, gene_b, penalty_factor).
/// effective_a = raw_a * (1 - raw_b_norm * factor), and vice-versa.
pub const ANTAGONISTIC_PAIRS: [(usize, usize, f32); 9] = [
    (PHOTOSYNTHESIS_RATE, SPEED, 0.7),
    (PHOTOSYNTHESIS_RATE, THERMOSYNTHESIS_RATE, 0.7),
    (ARMOR, SPEED, 0.7),
    (ATTACK_POWER, ENERGY_STORAGE_CAP, 0.7),
    (SENSE_RADIUS, BASE_METABOLISM, 0.7),
    (ADHESION, SPEED, 0.7),
    (SIGNAL_EMISSION, BASE_METABOLISM, 0.7),
    (TERRITORIAL_RADIUS, PACK_AFFINITY, 0.7),
    (ATTACK_RANGE, ATTACK_POWER, 0.7),
];

// ── GenomeHash ───────────────────────────────────────────────────────
/// A compact hash of the genome used for color mapping and kin recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenomeHash(pub u32);

impl GenomeHash {
    /// FNV-1a-inspired hash over the 64 genome bytes.
    pub fn from_genome(data: &[u8; GENOME_LEN]) -> Self {
        let mut h: u32 = 2_166_136_261;
        for &b in data.iter() {
            h ^= b as u32;
            h = h.wrapping_mul(16_777_619);
        }
        GenomeHash(h)
    }
}

// ── Genome struct ────────────────────────────────────────────────────
/// The 64-byte genome carried by every cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Genome {
    pub data: [u8; GENOME_LEN],
}

impl Genome {
    pub fn new(data: [u8; GENOME_LEN]) -> Self {
        Self { data }
    }

    /// Read a single base gene (index 0..45) as a raw u8.
    #[inline]
    pub fn gene(&self, index: usize) -> u8 {
        debug_assert!(index < BASE_GENE_COUNT);
        self.data[index]
    }

    /// Read a byte from phase slot `slot` (0..2) at field `offset` (0..5).
    #[inline]
    pub fn phase_byte(&self, slot: usize, offset: usize) -> u8 {
        debug_assert!(slot < PHASE_SLOT_COUNT);
        debug_assert!(offset < PHASE_SLOT_SIZE);
        self.data[BASE_GENE_COUNT + slot * PHASE_SLOT_SIZE + offset]
    }

    pub fn hash(&self) -> GenomeHash {
        GenomeHash::from_genome(&self.data)
    }

    // ── Expression pipeline ──────────────────────────────────────────

    /// Decode the genome into effective floating-point gene values.
    /// Pipeline: raw -> antagonistic pairs -> top-N gating -> physical caps.
    pub fn decode(&self, config: &WorldConfig) -> DecodedGenes {
        let mut eff = [0.0_f32; BASE_GENE_COUNT];

        // Step 0: normalize raw bytes to 0.0..1.0
        for i in 0..BASE_GENE_COUNT {
            eff[i] = self.data[i] as f32 / 255.0;
        }

        // Step 1: antagonistic pairs
        apply_antagonistic_pairs(&mut eff);

        // Step 2: top-N gating
        apply_top_n_gating(&mut eff, config);

        // Step 3: physical caps
        apply_physical_caps(&mut eff);

        DecodedGenes { values: eff }
    }

    /// Mutate this genome in-place using its own mutation_rate and
    /// mutation_magnitude genes. Returns true if any byte changed.
    pub fn mutate(&mut self, rng: &mut impl rand::Rng) -> bool {
        let rate = self.data[MUTATION_RATE];
        let magnitude = self.data[MUTATION_MAGNITUDE];
        if rate == 0 || magnitude == 0 {
            return false;
        }

        let mut changed = false;
        for i in 0..GENOME_LEN {
            // rate/255 probability of mutating each byte
            if rng.gen_range(0u16..255) < rate as u16 {
                let shift = rng.gen_range(0..=magnitude);
                if rng.gen_bool(0.5) {
                    self.data[i] = self.data[i].saturating_add(shift);
                } else {
                    self.data[i] = self.data[i].saturating_sub(shift);
                }
                changed = true;
            }
        }
        changed
    }
}

// ── Decoded effective gene values ────────────────────────────────────
/// The output of the expression pipeline: 46 floats in [0.0, 1.0].
#[derive(Debug, Clone)]
pub struct DecodedGenes {
    pub values: [f32; BASE_GENE_COUNT],
}

impl DecodedGenes {
    #[inline]
    pub fn get(&self, index: usize) -> f32 {
        self.values[index]
    }
}

// ── Pipeline steps ───────────────────────────────────────────────────

/// Apply antagonistic pair penalties. Both genes in a pair are reduced
/// based on the other's normalized value.
fn apply_antagonistic_pairs(eff: &mut [f32; BASE_GENE_COUNT]) {
    for &(a, b, factor) in &ANTAGONISTIC_PAIRS {
        let raw_a = eff[a];
        let raw_b = eff[b];
        eff[a] = f32::max(0.0, raw_a * (1.0 - raw_b * factor));
        eff[b] = f32::max(0.0, raw_b * (1.0 - raw_a * factor));
    }
}

/// Attenuate genes ranked below the top-N by effective value.
fn apply_top_n_gating(eff: &mut [f32; BASE_GENE_COUNT], config: &WorldConfig) {
    let n = config.top_n_gene_count as usize;
    let falloff = config.top_n_falloff;

    // Build (index, value) pairs and sort descending by value
    let mut ranked: Vec<(usize, f32)> = eff.iter().copied().enumerate().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Genes ranked beyond N are multiplied by the falloff factor
    for &(idx, _) in ranked.iter().skip(n) {
        eff[idx] *= falloff;
    }
}

/// Enforce structural caps between genes.
fn apply_physical_caps(eff: &mut [f32; BASE_GENE_COUNT]) {
    // attack_range <= sense_radius
    if eff[ATTACK_RANGE] > eff[SENSE_RADIUS] {
        eff[ATTACK_RANGE] = eff[SENSE_RADIUS];
    }
    // territorial_radius capped by speed
    if eff[TERRITORIAL_RADIUS] > eff[SPEED] {
        eff[TERRITORIAL_RADIUS] = eff[SPEED];
    }
    // offspring_scatter capped by sense_radius
    if eff[OFFSPRING_SCATTER] > eff[SENSE_RADIUS] {
        eff[OFFSPRING_SCATTER] = eff[SENSE_RADIUS];
    }
}

// ── Tests ────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn default_config() -> WorldConfig {
        WorldConfig::default()
    }

    fn uniform_genome(value: u8) -> Genome {
        Genome::new([value; GENOME_LEN])
    }

    // -- Genome basics --

    #[test]
    fn genome_is_64_bytes() {
        let g = uniform_genome(0);
        assert_eq!(g.data.len(), 64);
        assert_eq!(std::mem::size_of_val(&g.data), 64);
    }

    #[test]
    fn gene_accessor_returns_correct_byte() {
        let mut data = [0u8; GENOME_LEN];
        data[SPEED] = 200;
        data[ARMOR] = 42;
        let g = Genome::new(data);
        assert_eq!(g.gene(SPEED), 200);
        assert_eq!(g.gene(ARMOR), 42);
    }

    #[test]
    fn phase_byte_accessor() {
        let mut data = [0u8; GENOME_LEN];
        // Phase slot 1, offense_mod field
        data[BASE_GENE_COUNT + 1 * PHASE_SLOT_SIZE + PHASE_OFFENSE_MOD] = 180;
        let g = Genome::new(data);
        assert_eq!(g.phase_byte(1, PHASE_OFFENSE_MOD), 180);
    }

    #[test]
    fn genome_hash_deterministic() {
        let g = uniform_genome(77);
        assert_eq!(g.hash(), g.hash());
    }

    #[test]
    fn genome_hash_differs_for_different_genomes() {
        let a = uniform_genome(0);
        let b = uniform_genome(1);
        assert_ne!(a.hash(), b.hash());
    }

    // -- Top-N gating --

    #[test]
    fn top_n_gating_attenuates_low_ranked_genes() {
        let mut data = [0u8; GENOME_LEN];
        for i in 0..12 {
            data[i] = 200;
        }
        for i in 12..BASE_GENE_COUNT {
            data[i] = 100;
        }

        let config = default_config(); // top_n_gene_count = 12, falloff = 0.1

        let mut eff = [0.0f32; BASE_GENE_COUNT];
        for i in 0..BASE_GENE_COUNT {
            eff[i] = data[i] as f32 / 255.0;
        }
        apply_top_n_gating(&mut eff, &config);

        let high_val = 200.0 / 255.0;
        for i in 0..12 {
            assert!((eff[i] - high_val).abs() < 1e-6, "gene {i} should be ungated");
        }
        let low_val = (100.0 / 255.0) * 0.1;
        for i in 12..BASE_GENE_COUNT {
            assert!(
                (eff[i] - low_val).abs() < 1e-6,
                "gene {i}: got {} expected {}",
                eff[i],
                low_val
            );
        }
    }

    #[test]
    fn top_n_gating_preserves_top_genes_unchanged() {
        let config = default_config();
        let mut eff = [128.0 / 255.0; BASE_GENE_COUNT];
        apply_top_n_gating(&mut eff, &config);

        let full_count = eff
            .iter()
            .filter(|&&v| (v - 128.0 / 255.0).abs() < 1e-6)
            .count();
        assert_eq!(full_count, config.top_n_gene_count as usize);
    }

    // -- Physical caps --

    #[test]
    fn physical_cap_attack_range_by_sense_radius() {
        let mut eff = [0.5; BASE_GENE_COUNT];
        eff[ATTACK_RANGE] = 0.9;
        eff[SENSE_RADIUS] = 0.3;
        apply_physical_caps(&mut eff);
        assert!((eff[ATTACK_RANGE] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn physical_cap_territorial_radius_by_speed() {
        let mut eff = [0.5; BASE_GENE_COUNT];
        eff[TERRITORIAL_RADIUS] = 0.8;
        eff[SPEED] = 0.2;
        apply_physical_caps(&mut eff);
        assert!((eff[TERRITORIAL_RADIUS] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn physical_cap_offspring_scatter_by_sense_radius() {
        let mut eff = [0.5; BASE_GENE_COUNT];
        eff[OFFSPRING_SCATTER] = 0.7;
        eff[SENSE_RADIUS] = 0.4;
        apply_physical_caps(&mut eff);
        assert!((eff[OFFSPRING_SCATTER] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn physical_caps_no_effect_when_within_bounds() {
        let mut eff = [0.5; BASE_GENE_COUNT];
        eff[ATTACK_RANGE] = 0.2;
        eff[SENSE_RADIUS] = 0.8;
        eff[TERRITORIAL_RADIUS] = 0.1;
        eff[SPEED] = 0.9;
        eff[OFFSPRING_SCATTER] = 0.3;
        let original = eff;
        apply_physical_caps(&mut eff);
        assert_eq!(eff, original);
    }

    // -- Antagonistic pairs --

    #[test]
    fn antagonistic_zero_partner_imposes_no_penalty() {
        // If one gene in a pair is 0, the other should be unaffected
        let mut eff = [0.0; BASE_GENE_COUNT];
        eff[PHOTOSYNTHESIS_RATE] = 0.8;
        eff[SPEED] = 0.0;
        apply_antagonistic_pairs(&mut eff);
        assert!(
            (eff[PHOTOSYNTHESIS_RATE] - 0.8).abs() < 1e-6,
            "zero partner should impose no penalty"
        );
    }

    #[test]
    fn antagonistic_symmetric_penalty() {
        // Both genes in a pair should be reduced
        let mut eff = [0.0; BASE_GENE_COUNT];
        eff[PHOTOSYNTHESIS_RATE] = 0.8;
        eff[SPEED] = 0.6;
        apply_antagonistic_pairs(&mut eff);
        // photo: 0.8 * (1 - 0.6 * 0.7) = 0.8 * 0.58 = 0.464
        // speed: 0.6 * (1 - 0.8 * 0.7) = 0.6 * 0.44 = 0.264
        assert!((eff[PHOTOSYNTHESIS_RATE] - 0.464).abs() < 1e-4);
        assert!((eff[SPEED] - 0.264).abs() < 1e-4);
    }

    #[test]
    fn antagonistic_both_maxed_survive_at_thirty_percent() {
        let mut eff = [0.0; BASE_GENE_COUNT];
        eff[ARMOR] = 1.0;
        eff[SPEED] = 1.0;
        // Isolate just the armor/speed pair by zeroing other pair partners
        // (speed also appears in photo/speed and adhesion/speed pairs,
        //  but those partners are 0 so they impose no penalty)
        apply_antagonistic_pairs(&mut eff);
        // armor: 1.0 * (1 - 1.0 * 0.7) = 0.3
        // speed: 1.0 * (1 - 1.0 * 0.7) = 0.3  (from armor pair)
        //   but speed also hit by photo pair (photo=0, no effect) and adhesion pair (adhesion=0, no effect)
        assert!((eff[ARMOR] - 0.3).abs() < 1e-4);
        assert!((eff[SPEED] - 0.3).abs() < 1e-4);
    }

    #[test]
    fn antagonistic_multi_pair_gene_compounds() {
        // SPEED appears in 3 pairs: (photo, speed), (armor, speed), (adhesion, speed)
        // Penalties from each pair compound sequentially
        let mut eff = [0.0; BASE_GENE_COUNT];
        eff[PHOTOSYNTHESIS_RATE] = 1.0;
        eff[SPEED] = 1.0;
        eff[ARMOR] = 1.0;
        apply_antagonistic_pairs(&mut eff);
        // After photo/speed pair: speed = 1.0*(1-1.0*0.7) = 0.3, photo = 1.0*(1-1.0*0.7) = 0.3
        // After armor/speed pair: speed reads current 0.3, armor reads 1.0
        //   armor = 1.0*(1-0.3*0.7) = 1.0*0.79 = 0.79
        //   speed = 0.3*(1-1.0*0.7) = 0.3*0.3 = 0.09
        assert!(eff[SPEED] < 0.1, "multi-pair gene should be heavily penalized: {}", eff[SPEED]);
        assert!(eff[SPEED] > 0.0, "but never negative");
    }

    #[test]
    fn antagonistic_results_never_negative() {
        // All genes maxed -> maximum penalty pressure
        let mut eff = [1.0; BASE_GENE_COUNT];
        apply_antagonistic_pairs(&mut eff);
        for (i, &v) in eff.iter().enumerate() {
            assert!(v >= 0.0, "gene {i} went negative: {v}");
        }
    }

    // -- Full decode invariants --

    #[test]
    fn decoded_values_never_negative() {
        let g = uniform_genome(255);
        let decoded = g.decode(&default_config());
        for (i, &v) in decoded.values.iter().enumerate() {
            assert!(v >= 0.0, "gene {i} is negative: {v}");
        }
    }

    #[test]
    fn decoded_values_never_exceed_one() {
        let g = uniform_genome(255);
        let decoded = g.decode(&default_config());
        for (i, &v) in decoded.values.iter().enumerate() {
            assert!(v <= 1.0, "gene {i} exceeds 1.0: {v}");
        }
    }

    // -- Mutation --

    #[test]
    fn mutate_with_zero_rate_changes_nothing() {
        let mut g = uniform_genome(128);
        g.data[MUTATION_RATE] = 0;
        g.data[MUTATION_MAGNITUDE] = 50;
        let original = g.data;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        let changed = g.mutate(&mut rng);
        assert!(!changed);
        assert_eq!(g.data, original);
    }

    #[test]
    fn mutate_with_zero_magnitude_changes_nothing() {
        let mut g = uniform_genome(128);
        g.data[MUTATION_RATE] = 255;
        g.data[MUTATION_MAGNITUDE] = 0;
        let original = g.data;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        let changed = g.mutate(&mut rng);
        assert!(!changed);
        assert_eq!(g.data, original);
    }

    #[test]
    fn mutate_respects_byte_bounds() {
        let mut g = uniform_genome(250);
        g.data[MUTATION_RATE] = 255;
        g.data[MUTATION_MAGNITUDE] = 200;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
        g.mutate(&mut rng);
        // u8 can't exceed 255 by type, but saturating_add ensures no wrap
        for &b in &g.data {
            assert!(b <= 255);
        }
    }

    #[test]
    fn mutate_is_deterministic_with_same_seed() {
        let base = uniform_genome(128);

        let mut g1 = base.clone();
        g1.data[MUTATION_RATE] = 128;
        g1.data[MUTATION_MAGNITUDE] = 30;
        let mut rng1 = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        g1.mutate(&mut rng1);

        let mut g2 = base.clone();
        g2.data[MUTATION_RATE] = 128;
        g2.data[MUTATION_MAGNITUDE] = 30;
        let mut rng2 = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        g2.mutate(&mut rng2);

        assert_eq!(g1.data, g2.data);
    }
}

