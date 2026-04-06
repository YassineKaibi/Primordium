// Phase evaluation, hysteresis tracking, modifier computation

use crate::sim::genome::{
    BASE_GENE_COUNT, DecodedGenes, Genome, PHASE_DEFENSE_MOD, PHASE_EFFICIENCY_MOD,
    PHASE_MOBILITY_MOD, PHASE_OFFENSE_MOD, PHASE_SLOT_COUNT, PHASE_TRIGGER_CONDITION,
    PHASE_TRIGGER_THRESHOLD,
};

// ── Trigger conditions ──────────────────────────────────────────────

/// Environmental or internal condition that can activate a phase slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TriggerCondition {
    EnergyLow = 0,
    EnergyHigh = 1,
    ThreatNearby = 2,
    KinNearby = 3,
    AgeMature = 4,
    NoFood = 5,
    Crowded = 6,
    Wounded = 7,
}

impl TriggerCondition {
    /// Map a raw genome byte to a trigger condition.
    /// Values 0-7 map directly; anything else wraps via modulo.
    pub fn from_byte(b: u8) -> Self {
        match b % 8 {
            0 => TriggerCondition::EnergyLow,
            1 => TriggerCondition::EnergyHigh,
            2 => TriggerCondition::ThreatNearby,
            3 => TriggerCondition::KinNearby,
            4 => TriggerCondition::AgeMature,
            5 => TriggerCondition::NoFood,
            6 => TriggerCondition::Crowded,
            7 => TriggerCondition::Wounded,
            _ => unreachable!(),
        }
    }
}

// ── Threshold decoding ──────────────────────────────────────────────

/// Hysteresis band percentages indexed by the 2-bit preset value.
const HYSTERESIS_BANDS: [f32; 4] = [0.0, 0.10, 0.25, 0.40];

/// Decoded threshold: the activation value and hysteresis band.
#[derive(Debug, Clone, Copy)]
pub struct DecodedThreshold {
    /// Threshold value normalized to 0.0..1.0
    pub threshold: f32,
    /// Hysteresis band as a fraction of the threshold (0.0, 0.10, 0.25, 0.40)
    pub hysteresis_band: f32,
}

/// Decode the `trigger_threshold` byte from a phase slot.
/// Upper 6 bits → threshold (0-63 mapped to 0.0-1.0).
/// Lower 2 bits → hysteresis preset index.
pub fn decode_threshold(byte: u8) -> DecodedThreshold {
    let raw_threshold = (byte >> 2) as f32 / 63.0;
    let preset = (byte & 0b11) as usize;
    DecodedThreshold {
        threshold: raw_threshold,
        hysteresis_band: HYSTERESIS_BANDS[preset],
    }
}

// ── Sense context for phase evaluation ──────────────────────────────

/// Information the phase system needs from the cell and its surroundings.
/// Populated by the sensing phase before phase evaluation.
pub struct PhaseInput {
    /// Cell's current energy as a fraction of its storage cap (0.0..1.0)
    pub energy_fraction: f32,
    /// Number of aggressive non-kin within sense radius
    pub threat_count: u32,
    /// Number of genetic kin within sense radius
    pub kin_count: u32,
    /// Cell age in ticks
    pub age: u32,
    /// Whether any energy source was detected nearby
    pub food_nearby: bool,
    /// Total neighbor count within sense radius
    pub neighbor_count: u32,
    /// Ticks since last damage (u32::MAX if never damaged)
    pub ticks_since_damage: u32,
    /// Effective sense radius in tiles (from decoded sense_radius gene)
    pub sense_radius: u32,
    /// Maturity age in ticks, derived from the maturity_age gene (0 = always mature)
    pub maturity_threshold: u32,
    /// How long the cell remembers events, in ticks (from memory_length gene)
    pub memory_length: u32,
}

// ── Phase evaluation ────────────────────────────────────────────────

/// Evaluate phase slots and return the new active phase.
///
/// Returns 0 for default (no phase active), or 1-3 for the matching slot.
/// Implements hysteresis: entering requires crossing the threshold,
/// exiting requires crossing threshold + band.
pub fn evaluate_phase(genome: &Genome, input: &PhaseInput, current_phase: u8) -> u8 {
    for slot in 0..PHASE_SLOT_COUNT {
        let condition =
            TriggerCondition::from_byte(genome.phase_byte(slot, PHASE_TRIGGER_CONDITION));
        let decoded = decode_threshold(genome.phase_byte(slot, PHASE_TRIGGER_THRESHOLD));

        let phase_id = (slot as u8) + 1; // 1-indexed
        let currently_in_this_phase = current_phase == phase_id;

        let condition_value = condition_strength(&condition, input);
        let is_active = if currently_in_this_phase {
            // Already in this phase: stay unless condition drops below
            // threshold - hysteresis_band * threshold
            let exit_threshold = decoded.threshold * (1.0 - decoded.hysteresis_band);
            condition_value >= exit_threshold
        } else {
            // Not in this phase: enter only if condition exceeds threshold
            condition_value >= decoded.threshold
        };

        if is_active {
            return phase_id;
        }
    }
    0 // default: no phase active
}

/// Map a trigger condition + cell state to a 0.0..1.0 strength value
/// that can be compared against the decoded threshold.
fn condition_strength(condition: &TriggerCondition, input: &PhaseInput) -> f32 {
    match condition {
        TriggerCondition::EnergyLow => 1.0 - input.energy_fraction,
        TriggerCondition::EnergyHigh => input.energy_fraction,
        TriggerCondition::ThreatNearby => {
            let area = (2.0 * input.sense_radius as f32 + 1.0).powi(2);
            let k = area * 0.2;
            input.threat_count as f32 / (input.threat_count as f32 + k)
        }
        TriggerCondition::KinNearby => {
            let area = (2.0 * input.sense_radius as f32 + 1.0).powi(2);
            let k = area * 0.4;
            input.kin_count as f32 / (input.kin_count as f32 + k)
        }
        TriggerCondition::AgeMature => {
            if input.maturity_threshold == 0 {
                1.0
            } else {
                (input.age as f32 / input.maturity_threshold as f32).clamp(0.0, 1.0)
            }
        }
        TriggerCondition::NoFood => {
            if input.food_nearby {
                0.0
            } else {
                1.0
            }
        }
        TriggerCondition::Crowded => {
            let area = (2.0 * input.sense_radius as f32 + 1.0).powi(2);
            let k = area * 0.3;
            input.neighbor_count as f32 / (input.neighbor_count as f32 + k)
        }
        TriggerCondition::Wounded => {
            let window = input.memory_length.max(1) as f32;
            1.0 - (input.ticks_since_damage as f32 / window).clamp(0.0, 1.0)
        }
    }
}

// ── Phase modifier application ──────────────────────────────────────

/// Gene group indices for phase modifier scaling.
/// Offense: attack_power, venom, attack_range, aggression_trigger
const OFFENSE_GENES: [usize; 4] = [12, 14, 15, 16];
/// Defense: armor, membrane, rigidity
const DEFENSE_GENES: [usize; 3] = [13, 31, 29];
/// Mobility: speed, chemotaxis_strength, flee_response, direction_noise
const MOBILITY_GENES: [usize; 4] = [6, 9, 10, 8];
/// Efficiency: photosynthesis_rate, thermosynthesis_rate, scavenge_ability, sense_radius
const EFFICIENCY_GENES: [usize; 4] = [0, 1, 3, 23];

/// Apply phase modifiers to decoded gene values. Mutates in place.
///
/// Modifier bytes use 128 as neutral:
/// - 0   → gene scaled to ~0% (full suppress)
/// - 128 → gene unchanged (neutral)
/// - 255 → gene scaled to ~200% (full boost), clamped to 1.0
///
/// If `phase_slot` is 0, no modifiers are applied (default phase).
pub fn apply_phase_modifiers(genes: &mut DecodedGenes, genome: &Genome, phase_slot: u8) {
    if phase_slot == 0 {
        return;
    }
    let slot = (phase_slot - 1) as usize;
    if slot >= PHASE_SLOT_COUNT {
        return;
    }

    let offense_mod = genome.phase_byte(slot, PHASE_OFFENSE_MOD);
    let defense_mod = genome.phase_byte(slot, PHASE_DEFENSE_MOD);
    let mobility_mod = genome.phase_byte(slot, PHASE_MOBILITY_MOD);
    let efficiency_mod = genome.phase_byte(slot, PHASE_EFFICIENCY_MOD);

    apply_modifier(&mut genes.values, &OFFENSE_GENES, offense_mod);
    apply_modifier(&mut genes.values, &DEFENSE_GENES, defense_mod);
    apply_modifier(&mut genes.values, &MOBILITY_GENES, mobility_mod);
    apply_modifier(&mut genes.values, &EFFICIENCY_GENES, efficiency_mod);
}

/// Scale gene values by a modifier byte. 128 = neutral (1.0x), 0 = zero, 255 ≈ 2.0x.
fn apply_modifier(values: &mut [f32; BASE_GENE_COUNT], gene_indices: &[usize], modifier: u8) {
    let scale = modifier as f32 / 128.0;
    for &idx in gene_indices {
        values[idx] = (values[idx] * scale).clamp(0.0, 1.0);
    }
}

// ── Tests ────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::genome::{GENOME_LEN, Genome};

    #[test]
    fn trigger_condition_from_byte_direct() {
        assert_eq!(TriggerCondition::from_byte(0), TriggerCondition::EnergyLow);
        assert_eq!(TriggerCondition::from_byte(3), TriggerCondition::KinNearby);
        assert_eq!(TriggerCondition::from_byte(7), TriggerCondition::Wounded);
    }

    #[test]
    fn trigger_condition_from_byte_wraps() {
        assert_eq!(TriggerCondition::from_byte(8), TriggerCondition::EnergyLow);
        assert_eq!(TriggerCondition::from_byte(15), TriggerCondition::Wounded);
        assert_eq!(TriggerCondition::from_byte(255), TriggerCondition::Wounded);
    }

    #[test]
    fn decode_threshold_max() {
        // 0b111111_00 = threshold 63/63 = 1.0, hysteresis preset 0 (none)
        let d = decode_threshold(0b1111_1100);
        assert!((d.threshold - 1.0).abs() < f32::EPSILON);
        assert!((d.hysteresis_band - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn decode_threshold_min() {
        // 0b000000_11 = threshold 0/63 = 0.0, hysteresis preset 3 (40%)
        let d = decode_threshold(0b0000_0011);
        assert!((d.threshold - 0.0).abs() < f32::EPSILON);
        assert!((d.hysteresis_band - 0.40).abs() < f32::EPSILON);
    }

    #[test]
    fn decode_threshold_mid_with_preset() {
        // 0b100000_01 = threshold 32/63 ≈ 0.508, hysteresis preset 1 (10%)
        let d = decode_threshold(0b1000_0001);
        assert!((d.threshold - 32.0 / 63.0).abs() < 0.001);
        assert!((d.hysteresis_band - 0.10).abs() < f32::EPSILON);
    }

    #[test]
    fn modifier_128_is_neutral() {
        let mut values = [0.5_f32; BASE_GENE_COUNT];
        let original = values;
        apply_modifier(&mut values, &OFFENSE_GENES, 128);
        for &idx in &OFFENSE_GENES {
            assert!(
                (values[idx] - original[idx]).abs() < f32::EPSILON,
                "Gene {idx} changed with neutral modifier"
            );
        }
    }

    #[test]
    fn modifier_0_suppresses() {
        let mut values = [0.8_f32; BASE_GENE_COUNT];
        apply_modifier(&mut values, &DEFENSE_GENES, 0);
        for &idx in &DEFENSE_GENES {
            assert!(
                values[idx].abs() < f32::EPSILON,
                "Gene {idx} not suppressed: {}",
                values[idx]
            );
        }
    }

    #[test]
    fn modifier_255_boosts_clamped() {
        let mut values = [0.8_f32; BASE_GENE_COUNT];
        apply_modifier(&mut values, &MOBILITY_GENES, 255);
        for &idx in &MOBILITY_GENES {
            // 0.8 * (255/128) ≈ 1.594 → clamped to 1.0
            assert!(
                (values[idx] - 1.0).abs() < f32::EPSILON,
                "Gene {idx} not clamped: {}",
                values[idx]
            );
        }
    }

    #[test]
    fn apply_phase_modifiers_slot_0_is_noop() {
        let genome = Genome::new([128; GENOME_LEN]);
        let config = crate::config::WorldConfig::default();
        let mut genes = genome.decode(&config);
        let before = genes.values;
        apply_phase_modifiers(&mut genes, &genome, 0);
        assert_eq!(genes.values, before);
    }

    // ── condition_strength tests ──

    fn base_input() -> PhaseInput {
        PhaseInput {
            energy_fraction: 0.5,
            threat_count: 0,
            kin_count: 0,
            age: 0,
            food_nearby: true,
            neighbor_count: 0,
            ticks_since_damage: u32::MAX,
            sense_radius: 2,
            maturity_threshold: 100,
            memory_length: 10,
        }
    }

    #[test]
    fn condition_energy_low_inverts_fraction() {
        let input = PhaseInput {
            energy_fraction: 0.2,
            ..base_input()
        };
        let s = condition_strength(&TriggerCondition::EnergyLow, &input);
        assert!((s - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn condition_energy_high_direct() {
        let input = PhaseInput {
            energy_fraction: 0.9,
            ..base_input()
        };
        let s = condition_strength(&TriggerCondition::EnergyHigh, &input);
        assert!((s - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn condition_threat_scales_with_sense_radius() {
        let small = PhaseInput {
            sense_radius: 1,
            threat_count: 4,
            ..base_input()
        };
        let large = PhaseInput {
            sense_radius: 3,
            threat_count: 4,
            ..base_input()
        };
        let s_small = condition_strength(&TriggerCondition::ThreatNearby, &small);
        let s_large = condition_strength(&TriggerCondition::ThreatNearby, &large);
        // Same threat count feels more significant with a smaller sense radius
        assert!(
            s_small > s_large,
            "small radius {s_small} should > large radius {s_large}"
        );
    }

    #[test]
    fn condition_no_food_binary() {
        let has_food = PhaseInput {
            food_nearby: true,
            ..base_input()
        };
        let no_food = PhaseInput {
            food_nearby: false,
            ..base_input()
        };
        assert!((condition_strength(&TriggerCondition::NoFood, &has_food)).abs() < f32::EPSILON);
        assert!(
            (condition_strength(&TriggerCondition::NoFood, &no_food) - 1.0).abs() < f32::EPSILON
        );
    }

    #[test]
    fn condition_age_mature_zero_threshold_always_mature() {
        let input = PhaseInput {
            maturity_threshold: 0,
            age: 5,
            ..base_input()
        };
        assert!(
            (condition_strength(&TriggerCondition::AgeMature, &input) - 1.0).abs() < f32::EPSILON
        );
    }

    #[test]
    fn condition_wounded_decays_over_memory() {
        let just_hit = PhaseInput {
            ticks_since_damage: 0,
            ..base_input()
        };
        let mid = PhaseInput {
            ticks_since_damage: 5,
            ..base_input()
        };
        let forgotten = PhaseInput {
            ticks_since_damage: 20,
            ..base_input()
        };
        assert!(
            (condition_strength(&TriggerCondition::Wounded, &just_hit) - 1.0).abs() < f32::EPSILON
        );
        assert!((condition_strength(&TriggerCondition::Wounded, &mid) - 0.5).abs() < f32::EPSILON);
        assert!((condition_strength(&TriggerCondition::Wounded, &forgotten)).abs() < f32::EPSILON);
    }

    // ── evaluate_phase hysteresis tests ──

    #[test]
    fn evaluate_phase_enters_on_threshold() {
        // Slot 0: EnergyLow trigger, threshold ~0.5, no hysteresis
        let mut data = [0u8; GENOME_LEN];
        data[46] = 0; // EnergyLow
        data[47] = 0b1000_0000; // threshold 32/63 ≈ 0.508, preset 0 (no hysteresis)
        let genome = Genome::new(data);

        // energy_fraction 0.3 → EnergyLow strength = 0.7 > 0.508 → enters phase 1
        let input = PhaseInput {
            energy_fraction: 0.3,
            ..base_input()
        };
        assert_eq!(evaluate_phase(&genome, &input, 0), 1);
    }

    /// Helper: disable all 3 phase slots by setting threshold to max.
    fn disable_all_slots(data: &mut [u8; GENOME_LEN]) {
        for slot in 0..3 {
            let base = 46 + slot * 6;
            data[base + 1] = 0b1111_1100; // threshold 1.0, no hysteresis
        }
    }

    #[test]
    fn evaluate_phase_does_not_enter_below_threshold() {
        let mut data = [0u8; GENOME_LEN];
        disable_all_slots(&mut data);
        data[46] = 0; // EnergyLow
        data[47] = 0b1000_0000; // threshold ≈ 0.508
        let genome = Genome::new(data);

        // energy_fraction 0.8 → EnergyLow strength = 0.2 < 0.508 → stays default
        let input = PhaseInput {
            energy_fraction: 0.8,
            ..base_input()
        };
        assert_eq!(evaluate_phase(&genome, &input, 0), 0);
    }

    #[test]
    fn evaluate_phase_hysteresis_holds_phase() {
        let mut data = [0u8; GENOME_LEN];
        disable_all_slots(&mut data);
        data[46] = 0; // EnergyLow
        data[47] = 0b1000_0010; // threshold ≈ 0.508, preset 2 (25%)
        let genome = Genome::new(data);

        // Exit threshold = 0.508 * (1.0 - 0.25) = 0.381
        // energy_fraction 0.55 → strength = 0.45 → above exit (0.381), below entry (0.508)
        // If already in phase 1, should STAY (hysteresis holds)
        let input = PhaseInput {
            energy_fraction: 0.55,
            ..base_input()
        };
        assert_eq!(
            evaluate_phase(&genome, &input, 1),
            1,
            "hysteresis should hold phase"
        );
        // If NOT in phase 1, should NOT enter
        assert_eq!(
            evaluate_phase(&genome, &input, 0),
            0,
            "should not enter below threshold"
        );
    }

    #[test]
    fn evaluate_phase_hysteresis_exits_below_band() {
        let mut data = [0u8; GENOME_LEN];
        disable_all_slots(&mut data);
        data[46] = 0; // EnergyLow
        data[47] = 0b1000_0010; // threshold ≈ 0.508, preset 2 (25%)
        let genome = Genome::new(data);

        // Exit threshold = 0.508 * 0.75 = 0.381
        // energy_fraction 0.7 → strength = 0.3 < 0.381 → exits
        let input = PhaseInput {
            energy_fraction: 0.7,
            ..base_input()
        };
        assert_eq!(
            evaluate_phase(&genome, &input, 1),
            0,
            "should exit below hysteresis band"
        );
    }

    #[test]
    fn evaluate_phase_first_match_wins() {
        // Slot 0: EnergyLow, threshold ≈ 0.508
        // Slot 1: EnergyLow, threshold ≈ 0.508 (same condition)
        let mut data = [0u8; GENOME_LEN];
        data[46] = 0;
        data[47] = 0b1000_0000;
        data[52] = 0;
        data[53] = 0b1000_0000;
        let genome = Genome::new(data);

        let input = PhaseInput {
            energy_fraction: 0.3,
            ..base_input()
        };
        // Both would match, but slot 0 wins → phase 1, not phase 2
        assert_eq!(evaluate_phase(&genome, &input, 0), 1);
    }

    // ── apply_phase_modifiers tests ──

    #[test]
    fn apply_phase_modifiers_boost_offense() {
        // Build a genome where phase slot 0 has offense_mod = 200 (boost)
        // and all other mods = 128 (neutral)
        let mut data = [128u8; GENOME_LEN];
        // Phase slot 0 starts at byte 46
        data[46] = 0; // trigger_condition
        data[47] = 0b1000_0000; // trigger_threshold
        data[48] = 200; // offense_mod (boost)
        data[49] = 128; // defense_mod (neutral)
        data[50] = 128; // mobility_mod (neutral)
        data[51] = 128; // efficiency_mod (neutral)

        let genome = Genome::new(data);
        let config = crate::config::WorldConfig::default();
        let mut genes = genome.decode(&config);
        let before_offense = OFFENSE_GENES.map(|i| genes.values[i]);

        apply_phase_modifiers(&mut genes, &genome, 1); // slot 0 → phase_slot 1

        for (j, &idx) in OFFENSE_GENES.iter().enumerate() {
            let expected = (before_offense[j] * 200.0 / 128.0).clamp(0.0, 1.0);
            assert!(
                (genes.values[idx] - expected).abs() < 0.001,
                "Offense gene {idx}: expected {expected}, got {}",
                genes.values[idx]
            );
        }
    }
}
