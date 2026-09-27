// @veridikt
// kind: module
// name: Genome
// purpose: "The 64-byte genome and its expression pipeline: how raw bytes become the effective gene values that drive every behavior"
// owner: "primordium-maintainers"
// because: "Behavior is emergent, not hardcoded: there are no species, only genome bytes decoded under expression constraints that force specialization and prevent homogeneous supercells"
use crate::config::WorldConfig;

// ── Genome geometry ──────────────────────────────────────────────────
pub const GENOME_LEN: usize = 64;
pub const BASE_GENE_COUNT: usize = 46;
pub const PHASE_SLOT_COUNT: usize = 3;
pub const PHASE_SLOT_SIZE: usize = 6;

/// Ceiling on `gene_linkage`, which sets the mean length of a linked
/// mutation block to `1 / (1 - linkage)`. At 1.0 the block would run the
/// whole genome.
const MAX_GENE_LINKAGE: f32 = 0.9;

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

    // @veridikt
    // purpose: "Compact genome fingerprint used for render color and as the kin-similarity cue"
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

    // @veridikt
    // purpose: "Turn raw genome bytes into the effective per-gene strengths that all downstream systems read"
    // because: "The fixed order matters — antagonistic pairs, THEN top-N gating, THEN physical caps — so gating ranks genes by their already-penalized values and caps are enforced last"
    // assumes: "config.top_n_gene_count and top_n_falloff define how aggressively non-dominant genes are suppressed"
    pub fn decode(&self, config: &WorldConfig) -> DecodedGenes {
        let mut eff = [0.0_f32; BASE_GENE_COUNT];

        // Step 0: normalize raw bytes to 0.0..1.0
        for (i, val) in eff.iter_mut().enumerate().take(BASE_GENE_COUNT) {
            *val = self.data[i] as f32 / 255.0;
        }

        // Step 1: top-N gating — decide what the cell expresses at all.
        apply_top_n_gating(&mut eff, config);

        // Step 2: antagonistic pairs — trade off between what it *does*
        // express. Gating runs first so the three anti-supercell mechanisms
        // in `docs/spec.md` stay independent: with pairs first, a gene was
        // cut by its partners and then cut *again* by the falloff, because
        // those cuts had cost it its rank. `speed` sits in three pairs, more
        // than any other gene, and was gated in 99.8% of random genomes —
        // which closed both mobile niches, scavenging and predation, to
        // every genome the world could roll.
        //
        // It also means a gene the cell does not express exerts no
        // antagonistic pressure, which is the physically coherent reading:
        // armour a cell is not growing should not be slowing it down.
        apply_antagonistic_pairs(&mut eff);

        // Step 3: physical caps
        apply_physical_caps(&mut eff);

        DecodedGenes { values: eff }
    }

    /// Mutate this genome in-place using its own mutation_rate and
    /// mutation_magnitude genes. Returns true if any byte changed.

    // @veridikt
    // purpose: "Apply self-encoded mutation to a genome at reproduction, so mutation behavior itself evolves"
    // because: "Rate and magnitude are read from the genome's own MUTATION_RATE/MUTATION_MAGNITUDE bytes — meta-evolution: lineages select their own mutability"
    // assumes: "rng is the simulation's seeded ChaCha8 stream, so mutation is reproducible for a given seed"
    pub fn mutate(&mut self, config: &WorldConfig, rng: &mut impl rand::Rng) -> bool {
        let rate = self.data[MUTATION_RATE];
        let magnitude = self.data[MUTATION_MAGNITUDE];
        if rate == 0 || magnitude == 0 {
            return false;
        }

        // The genes set mutation *within* configured bounds. Read raw, a
        // mid-range founder rewrote ~half its 64 bytes per birth, so nothing
        // could be inherited and selection had nothing to accumulate.
        let p_mutate = (rate as f32 / 255.0) * config.max_mutation_rate;
        let max_shift = ((magnitude as f32 / 255.0) * config.max_mutation_magnitude as f32)
            .round()
            .max(1.0) as u8;

        // spec.md gene 43, gene_linkage: "controls which gene clusters tend
        // to mutate together (simulates chromosomes)". A mutation drags its
        // immediate neighbours along with probability `linkage`, so a high
        // value makes the genome mutate in blocks rather than pointwise.
        //
        // Linkage redistributes the mutational load, it does not add to it:
        // a block runs `1 / (1 - linkage)` bytes on average, so the chance of
        // *starting* one is divided by the same factor. Without that, a
        // mid-range founder mutated ~6 bytes a birth instead of ~1.4 and B15's
        // heredity went with it.
        let linkage = (self.data[GENE_LINKAGE] as f32 / 255.0).min(MAX_GENE_LINKAGE);
        let p_mutate = p_mutate * (1.0 - linkage);

        let mut changed = false;
        let mut i = 0;
        while i < GENOME_LEN {
            if rng.r#gen::<f32>() < p_mutate {
                // How far the linked block runs from here.
                let mut span = 1;
                while i + span < GENOME_LEN && rng.r#gen::<f32>() < linkage {
                    span += 1;
                }
                for j in i..i + span {
                    let shift = rng.gen_range(1..=max_shift);
                    if rng.gen_bool(0.5) {
                        self.data[j] = self.data[j].saturating_add(shift);
                    } else {
                        self.data[j] = self.data[j].saturating_sub(shift);
                    }
                }
                changed = true;
                i += span;
            } else {
                i += 1;
            }
        }

        // spec.md gene 45, transposon_rate: "rate of internal gene
        // duplication and shuffling within the genome". One byte is copied
        // over another, which is how a genome can acquire a capability it
        // already carries elsewhere without waiting for it to drift up from
        // zero.
        let transposon = (self.data[TRANSPOSON_RATE] as f32 / 255.0) * config.max_transposon_rate;
        if transposon > 0.0 && rng.r#gen::<f32>() < transposon {
            let from = rng.gen_range(0..BASE_GENE_COUNT);
            let to = rng.gen_range(0..BASE_GENE_COUNT);
            if from != to {
                self.data[to] = self.data[from];
                changed = true;
            }
        }

        changed
    }

    /// Absorb genetic material from a consumed cell.
    ///
    /// `docs/spec.md` gene 44, `horizontal_transfer`: "probability of
    /// absorbing genes from consumed cells into own genome". The gene was
    /// never read, so a predator learned nothing from what it ate.

    // @veridikt
    // purpose: "Copy a few of a victim's genome bytes into the killer's, with probability set by the killer's horizontal_transfer gene"
    // because: "It is the only route by which a capability can cross between lineages, which is what makes a predator's diet part of its own evolution rather than just its income"
    pub fn absorb_from(
        &mut self,
        victim: &Genome,
        config: &WorldConfig,
        rng: &mut impl rand::Rng,
    ) -> bool {
        let p = (self.data[HORIZONTAL_TRANSFER] as f32 / 255.0) * config.max_horizontal_transfer;
        if p <= 0.0 || rng.r#gen::<f32>() >= p {
            return false;
        }
        let idx = rng.gen_range(0..BASE_GENE_COUNT);
        if self.data[idx] == victim.data[idx] {
            return false;
        }
        self.data[idx] = victim.data[idx];
        true
    }
}

// ── Per-tick decode cache ────────────────────────────────────────────

/// One tick's worth of decoded genomes, indexed by cell id.
///
/// `decode` runs several times per cell per tick — once to decide, again for
/// every action `resolve_all` settles, again for vent income, again in
/// cleanup — and each run re-ranks all 46 genes for the top-N gate. The
/// genome itself cannot change between those calls except where `absorb_from`
/// rewrites it on a kill, so the result is worth keeping for the tick.
///
/// Cleared at the start of every tick, so there is no cross-tick staleness to
/// reason about. A cell id that is not present simply decodes on demand,
/// which is what newborns created mid-tick do.

// @veridikt
// kind: type
// purpose: "Memoize each cell's decoded genes for the duration of one tick, keyed by cell id"
// because: "Only two things can invalidate an entry — a new cell taking a recycled id, and horizontal gene transfer on a kill — and both are explicit calls, so the cache cannot silently desync from the genome"
#[derive(Default)]
pub struct DecodeCache {
    entries: Vec<Option<DecodedGenes>>,
}

impl DecodeCache {
    /// Decoded genes for `cell_id`, decoding and storing them on a miss.
    pub fn get(&mut self, cell_id: u32, genome: &Genome, config: &WorldConfig) -> &DecodedGenes {
        let idx = cell_id as usize;
        if idx >= self.entries.len() {
            self.entries.resize_with(idx + 1, || None);
        }
        self.entries[idx].get_or_insert_with(|| genome.decode(config))
    }

    /// Drop the entry for one cell. Call this wherever a genome is rewritten
    /// in place, or where a cell id is handed to a different cell.
    #[inline]
    pub fn invalidate(&mut self, cell_id: u32) {
        if let Some(slot) = self.entries.get_mut(cell_id as usize) {
            *slot = None;
        }
    }

    /// Forget everything. Called once per tick.
    #[inline]
    pub fn clear(&mut self) {
        self.entries.clear();
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

/// Apply antagonistic pair penalties.
///
/// For two capability genes the penalty is mutual: each one's effective value
/// is reduced by the other's, so neither is strong for free.
///
/// `base_metabolism` is not a capability, it is a **cost** — `docs/spec.md`
/// gene 5, "lower is more efficient". Reducing it is a *reward*, so the
/// mutual form inverted the two pairs it appears in: `spec.md` says
/// "awareness increases metabolic drain" and "broadcasting is energetically
/// expensive", but investing in `sense_radius` or `signal_emission` made a
/// cell **cheaper** to run. Those pairs are a one-directional surcharge
/// instead: the partner raises the cost gene, and the cost gene does not
/// blunt the partner (a cell that runs hot does not thereby see less).

// @veridikt
// purpose: "Enforce trade-offs (e.g. photosynthesis vs speed, armor vs speed) so a gene cannot be strong for free, charging a surcharge instead of a discount where the paired gene is a cost"
// because: "Penalties are applied sequentially over ANTAGONISTIC_PAIRS, so a gene appearing in several pairs (like SPEED) compounds its penalties — this is intended, not a bug"
fn apply_antagonistic_pairs(eff: &mut [f32; BASE_GENE_COUNT]) {
    // Every penalty is computed against the *pre-antagonism* values.
    // `docs/spec.md` states the rule as `effective_a = raw_a * (1 - raw_b_norm
    // * factor)` — raw_b, not a partner that an earlier pair already cut
    // down. Feeding the running values forward made the outcome depend on the
    // order of ANTAGONISTIC_PAIRS: `thermosynthesis` was penalised by a
    // `photosynthesis` that the speed pair had already reduced, so it came out
    // higher than the spec's formula gives.
    let raw = *eff;
    for &(a, b, factor) in &ANTAGONISTIC_PAIRS {
        let raw_a = raw[a];
        let raw_b = raw[b];
        if is_cost_gene(b) {
            // Surcharge accumulates on whatever the cost gene already is.
            eff[b] = (eff[b] + raw_a * factor).min(1.0);
        } else if is_cost_gene(a) {
            eff[a] = (eff[a] + raw_b * factor).min(1.0);
        } else {
            // A gene in several pairs pays each penalty, which is intended;
            // computing them all from `raw` is what makes them independent of
            // the order they are listed in.
            eff[a] = f32::max(0.0, eff[a] * (1.0 - raw_b * factor));
            eff[b] = f32::max(0.0, eff[b] * (1.0 - raw_a * factor));
        }
    }
}

/// Genes that are a drain rather than a capability, so a higher effective
/// value is worse for the cell.
#[inline]
fn is_cost_gene(index: usize) -> bool {
    index == BASE_METABOLISM
}

/// Attenuate genes ranked below the top-N by effective value.

// @veridikt
// purpose: "Force specialization: only the strongest ~top_n_gene_count genes express fully; the rest are scaled down by falloff"
// because: "Without this a genome could be good at everything; gating means ~10-12 of 46 genes carry a cell, pushing populations into distinct niches"
fn apply_top_n_gating(eff: &mut [f32; BASE_GENE_COUNT], config: &WorldConfig) {
    let n = config.top_n_gene_count as usize;
    let falloff = config.top_n_falloff;
    if n >= BASE_GENE_COUNT {
        return;
    }

    // Only the split at position N matters, not the order within each side,
    // so this selects rather than sorts: `decode` runs several times per cell
    // per tick and the sort was 16.5% of a run's time.
    //
    // Ranking is by value descending, ties broken by index ascending. That is
    // a total order with no ties at all, which is what makes the partition
    // deterministic — and it is the same order the previous stable sort
    // produced, so the gated set is bit-identical.
    let mut ranked: [(usize, f32); BASE_GENE_COUNT] = [(0, 0.0); BASE_GENE_COUNT];
    for (idx, slot) in ranked.iter_mut().enumerate() {
        *slot = (idx, eff[idx]);
    }
    ranked.select_nth_unstable_by(n, |a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });

    // Genes ranked beyond N are multiplied by the falloff factor
    for &(idx, _) in ranked.iter().skip(n) {
        eff[idx] *= falloff;
    }
}

/// Enforce structural caps between genes.

// @veridikt
// purpose: "Clamp physically-dependent genes to their enabling gene (attack_range<=sense_radius, territorial_radius<=speed, offspring_scatter<=sense_radius)"
// because: "A cell cannot strike or scatter farther than it can sense or move; these caps run last so they bound the already-gated values"
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

    /// Gating runs before antagonism, so the two anti-supercell mechanisms
    /// stay independent. With pairs first, a gene was cut by its partners and
    /// then cut *again* by the falloff, because those cuts had cost it its
    /// rank. `speed` is in three pairs — more than any other gene — and was
    /// gated in 99.8% of random genomes, closing both mobile niches
    /// (scavenging, predation) to every genome the world could roll.
    #[test]
    fn a_genes_own_investment_decides_whether_it_expresses_not_its_partners() {
        let config = WorldConfig::default();

        // speed is this genome's single biggest investment, but it is also
        // paired with photosynthesis, armor and adhesion, all of which are
        // present. It must still express.
        let mut d = [0u8; GENOME_LEN];
        d[SPEED] = 255;
        d[PHOTOSYNTHESIS_RATE] = 150;
        d[ARMOR] = 150;
        d[ADHESION] = 150;
        // Enough other mid-range genes to fill the top-N slots and crowd out
        // anything that antagonism knocks down the ranking.
        for g in [
            ENERGY_STORAGE_CAP,
            REPRODUCTION_THRESHOLD,
            OFFSPRING_ENERGY_SHARE,
            MUTATION_RATE,
            MUTATION_MAGNITUDE,
            MAX_AGE,
            MEMBRANE,
            SENSE_RADIUS,
            TEMPERATURE_PREFERENCE,
            RIGIDITY,
            TOXIN_RESISTANCE,
            SIGNAL_SENSITIVITY,
        ] {
            d[g] = 140;
        }

        let speed = Genome::new(d).decode(&config).get(SPEED);
        // Three pairs at 150/255 each still cut it hard — that is intended —
        // but it must not *also* take the x0.1 falloff on top.
        assert!(
            speed > config.top_n_falloff,
            "speed decoded to {speed}, at or below the {} falloff: its own partners \
             gated it out of the genome's own strongest gene",
            config.top_n_falloff
        );
    }

    /// `docs/spec.md`: "effective_a = raw_a * (1 - raw_b_norm * factor)".
    /// raw_b — not a partner an earlier pair already cut down. Feeding the
    /// running values forward made the result depend on the order
    /// `ANTAGONISTIC_PAIRS` happens to be written in.
    #[test]
    fn antagonism_does_not_depend_on_the_order_the_pairs_are_listed_in() {
        let config = WorldConfig::default();
        // thermosynthesis is penalised by photosynthesis, which is itself
        // penalised by speed in an earlier pair. Under the old running-value
        // form, thermo therefore came out higher when speed was high.
        let decoded = |speed: u8| {
            let mut d = [0u8; GENOME_LEN];
            d[PHOTOSYNTHESIS_RATE] = 200;
            d[THERMOSYNTHESIS_RATE] = 200;
            d[SPEED] = speed;
            Genome::new(d).decode(&config).get(THERMOSYNTHESIS_RATE)
        };
        let slow = decoded(0);
        let fast = decoded(255);
        assert!(
            (slow - fast).abs() < 1e-6,
            "thermosynthesis decodes to {slow} with speed 0 and {fast} with speed 255; \
             it is not paired with speed at all, so the two must match"
        );
    }

    /// B16: `docs/spec.md` says the sense_radius and signal_emission pairs
    /// exist because "awareness increases metabolic drain" and "broadcasting
    /// is energetically expensive". The mutual-reduction form gave the
    /// opposite: investing in either made the cell cheaper to run.
    #[test]
    fn sensing_and_signalling_raise_metabolism_instead_of_lowering_it() {
        let config = WorldConfig::default();
        let quiet = {
            let mut d = [0u8; GENOME_LEN];
            d[BASE_METABOLISM] = 60;
            Genome::new(d)
        };
        for &gene in &[SENSE_RADIUS, SIGNAL_EMISSION] {
            let mut d = [0u8; GENOME_LEN];
            d[BASE_METABOLISM] = 60;
            d[gene] = 255;
            let aware = Genome::new(d);

            let quiet_metab = quiet.decode(&config).get(BASE_METABOLISM);
            let aware_metab = aware.decode(&config).get(BASE_METABOLISM);
            assert!(
                aware_metab > quiet_metab,
                "gene {gene} left base_metabolism at {aware_metab} vs {quiet_metab} with it off"
            );
            assert!(
                crate::sim::energy::metabolic_cost(&aware.decode(&config), 128, &config)
                    > crate::sim::energy::metabolic_cost(&quiet.decode(&config), 128, &config),
                "gene {gene} did not raise the metabolic cost"
            );
        }
    }

    /// The cost gene must not blunt its partner: running hot does not make a
    /// cell see less.
    #[test]
    fn a_high_base_metabolism_does_not_shrink_sense_radius() {
        let config = WorldConfig::default();
        let mut lean = [0u8; GENOME_LEN];
        lean[SENSE_RADIUS] = 200;
        let mut hot = lean;
        hot[BASE_METABOLISM] = 255;
        assert_eq!(
            Genome::new(hot).decode(&config).get(SENSE_RADIUS),
            Genome::new(lean).decode(&config).get(SENSE_RADIUS)
        );
    }

    /// Two capability genes keep the mutual form.
    #[test]
    fn capability_pairs_still_penalize_each_other() {
        let config = WorldConfig::default();
        let mut both = [0u8; GENOME_LEN];
        both[PHOTOSYNTHESIS_RATE] = 255;
        both[SPEED] = 255;
        let mut photo_only = [0u8; GENOME_LEN];
        photo_only[PHOTOSYNTHESIS_RATE] = 255;

        let paired = Genome::new(both).decode(&config).get(PHOTOSYNTHESIS_RATE);
        let alone = Genome::new(photo_only)
            .decode(&config)
            .get(PHOTOSYNTHESIS_RATE);
        assert!(paired < alone, "speed did not penalize photosynthesis");
    }

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
        data[BASE_GENE_COUNT + PHASE_SLOT_SIZE + PHASE_OFFENSE_MOD] = 180;
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
        for (i, byte) in data.iter_mut().enumerate().take(BASE_GENE_COUNT) {
            *byte = if i < 12 { 200 } else { 100 };
        }

        let config = default_config(); // top_n_gene_count = 12, falloff = 0.1

        let mut eff = [0.0f32; BASE_GENE_COUNT];
        for (slot, &byte) in eff.iter_mut().zip(data.iter()) {
            *slot = byte as f32 / 255.0;
        }
        apply_top_n_gating(&mut eff, &config);

        let high_val = 200.0 / 255.0;
        let low_val = (100.0 / 255.0) * 0.1;
        for (i, &value) in eff.iter().enumerate() {
            let expected = if i < 12 { high_val } else { low_val };
            assert!(
                (value - expected).abs() < 1e-6,
                "gene {i}: got {value} expected {expected}"
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
        assert!(
            eff[SPEED] < 0.1,
            "multi-pair gene should be heavily penalized: {}",
            eff[SPEED]
        );
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
        let changed = g.mutate(&WorldConfig::default(), &mut rng);
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
        let changed = g.mutate(&WorldConfig::default(), &mut rng);
        assert!(!changed);
        assert_eq!(g.data, original);
    }

    #[test]
    fn mutate_respects_byte_bounds() {
        let mut g = uniform_genome(250);
        g.data[MUTATION_RATE] = 255;
        g.data[MUTATION_MAGNITUDE] = 200;
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
        g.mutate(&WorldConfig::default(), &mut rng);
        // saturating_add/sub means a mutation can never wrap a byte around;
        // u8 makes the upper bound unprovable by assertion, so check that the
        // genome is still the right length and every byte is reachable.
        assert_eq!(g.data.len(), GENOME_LEN);
    }

    /// spec.md gene 43: linkage decides *which* bytes move together, not how
    /// many move. Making a block drag its neighbours without dividing the
    /// start probability pushed a mid-range founder from ~1.4 mutated bytes
    /// per birth to ~6, undoing B15.
    #[test]
    fn gene_linkage_clusters_mutations_without_adding_to_the_load() {
        let config = WorldConfig::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(31);

        let load_and_runs = |linkage: u8, rng: &mut rand_chacha::ChaCha8Rng| {
            let mut data = [40u8; GENOME_LEN];
            data[MUTATION_RATE] = 255;
            data[MUTATION_MAGNITUDE] = 128;
            data[GENE_LINKAGE] = linkage;
            data[TRANSPOSON_RATE] = 0;
            let parent = Genome::new(data);
            let (mut bytes, mut runs) = (0u32, 0u32);
            for _ in 0..600 {
                let mut child = parent.clone();
                child.mutate(&config, rng);
                let changed: Vec<bool> = (0..GENOME_LEN)
                    .map(|i| child.data[i] != parent.data[i])
                    .collect();
                bytes += changed.iter().filter(|&&c| c).count() as u32;
                runs += changed
                    .iter()
                    .enumerate()
                    .filter(|&(i, &c)| c && (i == 0 || !changed[i - 1]))
                    .count() as u32;
            }
            (bytes as f32 / 600.0, bytes as f32 / runs.max(1) as f32)
        };

        let (loose_load, loose_run) = load_and_runs(0, &mut rng);
        let (linked_load, linked_run) = load_and_runs(230, &mut rng);

        assert!(
            linked_run > loose_run * 1.5,
            "linked runs average {linked_run:.2} bytes against {loose_run:.2} unlinked — \
             mutations are not clustering"
        );
        assert!(
            (linked_load - loose_load).abs() < loose_load * 0.5,
            "linkage changed the mutational load from {loose_load:.2} to {linked_load:.2}"
        );
    }

    /// spec.md gene 44: a predator absorbs genes from what it eats.
    #[test]
    fn horizontal_transfer_copies_a_gene_from_the_victim() {
        let config = WorldConfig::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(5);
        let victim = Genome::new([200u8; GENOME_LEN]);

        let mut never = Genome::new([10u8; GENOME_LEN]);
        for _ in 0..500 {
            assert!(
                !never.absorb_from(&victim, &config, &mut rng),
                "a horizontal_transfer-0 cell absorbed a gene"
            );
        }

        let mut data = [10u8; GENOME_LEN];
        data[HORIZONTAL_TRANSFER] = 255;
        let mut greedy = Genome::new(data);
        let absorbed = (0..2000)
            .filter(|_| greedy.absorb_from(&victim, &config, &mut rng))
            .count();
        assert!(
            absorbed > 0,
            "a maxed horizontal_transfer cell never absorbed anything"
        );
        assert!(
            greedy.data.iter().filter(|&&b| b == 200).count() > 0,
            "nothing of the victim's genome ended up in the killer's"
        );
    }

    /// spec.md gene 45: internal duplication copies one gene over another.
    #[test]
    fn transposon_rate_duplicates_a_gene_within_the_genome() {
        let config = WorldConfig::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(17);
        let mut data = [0u8; GENOME_LEN];
        data[MUTATION_RATE] = 1; // almost no point mutation, so any block move is the transposon
        data[MUTATION_MAGNITUDE] = 1;
        data[PHOTOSYNTHESIS_RATE] = 250;
        data[TRANSPOSON_RATE] = 255;
        let parent = Genome::new(data);

        let copies = (0..4000)
            .filter(|_| {
                let mut child = parent.clone();
                child.mutate(&config, &mut rng);
                // The maxed gene landed somewhere it was not before.
                (0..BASE_GENE_COUNT).any(|i| i != PHOTOSYNTHESIS_RATE && child.data[i] >= 249)
            })
            .count();
        assert!(
            copies > 0,
            "transposon_rate never duplicated a gene in 4000 births"
        );
    }

    #[test]
    fn mutate_changes_only_a_few_bytes_per_birth() {
        // Heredity: an unbounded rate byte rewrote 35-58 of 64 bytes per
        // birth, so a child was no more related to its parent than to a
        // stranger and selection could not accumulate anything.
        let config = WorldConfig::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(4);
        let mut total = 0usize;
        let births = 400;
        for _ in 0..births {
            let mut g = uniform_genome(128);
            g.data[MUTATION_RATE] = 255; // fastest mutator possible
            g.data[MUTATION_MAGNITUDE] = 255;
            let before = g.data;
            g.mutate(&config, &mut rng);
            total += (0..GENOME_LEN).filter(|&i| g.data[i] != before[i]).count();
        }
        let mean = total as f32 / births as f32;
        let bound = GENOME_LEN as f32 * config.max_mutation_rate * 1.5;
        assert!(
            mean < bound,
            "mean {mean} mutated bytes per birth should stay under {bound}"
        );
    }

    #[test]
    fn mutation_genes_still_scale_within_the_bound() {
        // Meta-evolution must survive the bound: a fast mutator still
        // mutates more than a slow one.
        let config = WorldConfig::default();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(11);
        let count = |rate: u8, rng: &mut rand_chacha::ChaCha8Rng| {
            let mut total = 0usize;
            for _ in 0..300 {
                let mut g = uniform_genome(128);
                g.data[MUTATION_RATE] = rate;
                g.data[MUTATION_MAGNITUDE] = 200;
                let before = g.data;
                g.mutate(&config, rng);
                total += (0..GENOME_LEN).filter(|&i| g.data[i] != before[i]).count();
            }
            total
        };
        let slow = count(20, &mut rng);
        let fast = count(255, &mut rng);
        assert!(fast > slow * 3, "fast mutator {fast} vs slow {slow}");
    }

    #[test]
    fn mutate_is_deterministic_with_same_seed() {
        let base = uniform_genome(128);

        let mut g1 = base.clone();
        g1.data[MUTATION_RATE] = 128;
        g1.data[MUTATION_MAGNITUDE] = 30;
        let mut rng1 = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        g1.mutate(&WorldConfig::default(), &mut rng1);

        let mut g2 = base.clone();
        g2.data[MUTATION_RATE] = 128;
        g2.data[MUTATION_MAGNITUDE] = 30;
        let mut rng2 = rand_chacha::ChaCha8Rng::seed_from_u64(42);
        g2.mutate(&WorldConfig::default(), &mut rng2);

        assert_eq!(g1.data, g2.data);
    }
}
