// Energy income (photo/thermo/scavenge), metabolic drain, starvation

use crate::config::WorldConfig;
use crate::sim::cell::Cell;
use crate::sim::genome::{self, DecodedGenes};

// ── Constants ──────────────────────────────────────────────────────

/// Maximum energy a fully-specialized photosynthesizer earns per tick
/// in full sunlight. Balances against metabolic_cost_exponent: a lean
/// genome (~2-4 cost/tick) thrives, a generalist (~8-10) cannot.
const PHOTO_MAX_INCOME: f32 = 4.0;

/// Fraction of consumed decay that becomes cell energy (energy sink).
const SCAVENGE_EFFICIENCY: f32 = 0.9;

// ── Photosynthesis ─────────────────────────────────────────────────

/// Calculate photosynthesis energy income for a cell.
///
/// `effective_rate`: decoded photosynthesis_rate gene (0.0..1.0)
/// `tile_sunlight`: sunlight value on the cell's tile (0..255)
pub fn photo_income(effective_rate: f32, tile_sunlight: u8) -> f32 {
    let sunlight_norm = tile_sunlight as f32 / 255.0;
    PHOTO_MAX_INCOME * effective_rate * sunlight_norm
}

// ── Thermosynthesis ────────────────────────────────────────────────

/// Calculate thermosynthesis energy income for a cell near a thermal vent.
///
/// `effective_rate`: decoded thermosynthesis_rate gene (0.0..1.0)
/// `vent_output`: energy emitted by this vent per tick (from config)
/// `adjacent_count`: number of cells adjacent to this vent (energy is shared)
/// `tick`: current simulation tick
/// `vent_cycle`: (active_ticks, dormant_ticks). (0, 0) = always on.
///
/// Returns energy gained this tick from the vent. Zero if vent is dormant
/// or cell has no thermosynthesis gene.
pub fn thermo_income(
    effective_rate: f32,
    vent_output: f32,
    adjacent_count: u32,
    tick: u64,
    vent_cycle: (u32, u32),
) -> f32 {
    if adjacent_count == 0 {
        return 0.0;
    }

    let (active, dormant) = vent_cycle;
    let period = active + dormant;
    let is_active = if period == 0 {
        true
    } else {
        let pos = (tick % period as u64) as u32;
        pos < active
    };

    if !is_active {
        return 0.0;
    }

    let per_cell_share = vent_output / adjacent_count as f32;
    effective_rate * per_cell_share
}

// ── Scavenge ───────────────────────────────────────────────────────

/// Calculate scavenge energy income from decay matter on a tile.
///
/// `effective_ability`: decoded scavenge_ability gene (0.0..1.0)
/// `tile_decay`: current decay energy on the cell's tile
///
/// Returns `(income, decay_consumed)` — energy gained and how much
/// decay to subtract from the tile. Decay consumed must not exceed
/// what's available.
pub fn scavenge_income(effective_ability: f32, tile_decay: f32) -> (f32, f32) {
    let decay_consumed = (effective_ability * tile_decay).min(tile_decay);
    (decay_consumed * SCAVENGE_EFFICIENCY, decay_consumed)
}

// ── Metabolic cost ─────────────────────────────────────────────────

/// Calculate total metabolic energy drain per tick.
///
/// Each gene's cost = `gene_value ^ exponent`, summed across all 46 genes.
/// The superlinear exponent (default 1.5) makes high gene values
/// disproportionately expensive — this is the core anti-supercell mechanic.
///
/// An additional penalty is applied for temperature mismatch between the
/// cell's `temperature_preference` gene and the local tile temperature.
///
/// `decoded`: effective gene values after all expression constraints
/// `tile_temperature`: local tile temperature (0..255)
/// `config`: world config (for exponent and temperature mismatch cost)
pub fn metabolic_cost(decoded: &DecodedGenes, tile_temperature: u8, config: &WorldConfig) -> f32 {
    let mut base_cost = 0.0_f32;
    for &value in decoded.values.iter() {
        base_cost += value.powf(config.metabolic_cost_exponent);
    }

    let temp_norm = tile_temperature as f32 / 255.0;
    let pref = decoded.get(genome::TEMPERATURE_PREFERENCE);
    let mismatch = (temp_norm - pref).abs();
    let penalty = mismatch * config.temperature_mismatch_cost;

    base_cost + penalty
}

// ── Venom & toxin damage ───────────────────────────────────────────

/// Calculate venom tick damage. Venom is applied by attackers and deals
/// damage over several ticks. Membrane gene reduces the damage taken.
///
/// `venom_damage`: raw damage per tick from the venom (stored on cell)
/// `membrane`: effective membrane gene (0.0..1.0), reduces damage
///
/// Returns energy to subtract from the poisoned cell this tick.
pub fn venom_tick_damage(venom_damage: u8, membrane: f32) -> f32 {
    (venom_damage as f32 * (1.0 - membrane)).max(0.0)
}

/// Calculate toxin tile damage. Cells on toxic tiles take damage each tick,
/// reduced by both toxin_resistance and membrane genes.
///
/// `tile_toxin`: toxin level on the cell's tile
/// `toxin_resistance`: effective toxin_resistance gene (0.0..1.0)
/// `membrane`: effective membrane gene (0.0..1.0)
pub fn toxin_tile_damage(tile_toxin: f32, toxin_resistance: f32, membrane: f32) -> f32 {
    (tile_toxin * (1.0 - toxin_resistance) * (1.0 - membrane * 0.85)).max(0.0)
}

// ── Energy update orchestrator ─────────────────────────────────────

/// Context needed to update a cell's energy for one tick.
/// Populated by the tick orchestrator before calling `update_energy`.
pub struct EnergyContext {
    /// Decoded gene values (after expression pipeline + phase modifiers)
    pub decoded: DecodedGenes,
    /// Sunlight on the cell's tile (0..255)
    pub tile_sunlight: u8,
    /// Temperature on the cell's tile (0..255)
    pub tile_temperature: u8,
    /// Toxin level on the cell's tile
    pub tile_toxin: f32,
    /// Decay energy on the cell's tile
    pub tile_decay: f32,
    /// Energy from nearby vent (0 if no vent adjacent). Pre-computed
    /// by the tick orchestrator which knows vent positions and sharing.
    pub vent_income: f32,
}

/// Result of an energy update — tells the caller what changed.
pub struct EnergyResult {
    /// How much decay was consumed from the tile
    pub decay_consumed: f32,
    /// Whether the cell died this tick
    pub died: bool,
}

/// Apply all energy income and costs to a cell for one tick.
/// Mutates `cell.energy` in place. Returns info the caller needs.
pub fn update_energy(cell: &mut Cell, ctx: &EnergyContext, config: &WorldConfig) -> EnergyResult {
    let photosynthesis_income = photo_income(
        ctx.decoded.get(genome::PHOTOSYNTHESIS_RATE),
        ctx.tile_sunlight,
    );
    let (scavenge_income, decay_consumed) =
        scavenge_income(ctx.decoded.get(genome::SCAVENGE_ABILITY), ctx.tile_decay);

    cell.energy += photosynthesis_income + ctx.vent_income + scavenge_income;

    cell.energy -= metabolic_cost(&ctx.decoded, ctx.tile_temperature, config);

    if cell.venom_ticks > 0 {
        cell.energy -= venom_tick_damage(cell.venom_damage, ctx.decoded.get(genome::MEMBRANE));
        cell.venom_ticks -= 1;
    }

    cell.energy -= toxin_tile_damage(
        ctx.tile_toxin,
        ctx.decoded.get(genome::TOXIN_RESISTANCE),
        ctx.decoded.get(genome::MEMBRANE),
    );

    let max_energy = ctx.decoded.get(genome::ENERGY_STORAGE_CAP) * 255.0;
    cell.energy = cell.energy.min(max_energy);

    let died = cell.energy <= 0.0;

    EnergyResult {
        decay_consumed,
        died,
    }
}

// ── Tests ───────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::genome::BASE_GENE_COUNT;

    // ── Photosynthesis tests ────────────────────────────────────────

    #[test]
    fn photo_zero_in_dark_tile() {
        // No sunlight → no income regardless of gene value
        assert!((photo_income(1.0, 0)).abs() < f32::EPSILON);
    }

    #[test]
    fn photo_zero_with_no_gene() {
        // Gene is 0 → no income regardless of sunlight
        assert!((photo_income(0.0, 255)).abs() < f32::EPSILON);
    }

    #[test]
    fn photo_proportional_to_sunlight() {
        let bright = photo_income(0.5, 200);
        let dim = photo_income(0.5, 50);
        assert!(bright > dim, "bright ({bright}) should exceed dim ({dim})");
    }

    #[test]
    fn photo_proportional_to_rate() {
        let high = photo_income(0.8, 128);
        let low = photo_income(0.2, 128);
        assert!(
            high > low,
            "high rate ({high}) should exceed low rate ({low})"
        );
    }

    #[test]
    fn photo_max_gives_reasonable_value() {
        let income = photo_income(1.0, 255);
        // Max rate + max sunlight should give meaningful but not absurd income
        assert!(income > 0.0);
        assert!(income <= 255.0, "income {income} seems too high");
    }

    // ── Thermosynthesis tests ───────────────────────────────────────

    #[test]
    fn thermo_zero_with_no_gene() {
        let income = thermo_income(0.0, 8.0, 1, 0, (0, 0));
        assert!(income.abs() < f32::EPSILON);
    }

    #[test]
    fn thermo_always_on_when_cycle_zero() {
        // (0, 0) = permanent vent
        let income = thermo_income(1.0, 8.0, 1, 999, (0, 0));
        assert!(income > 0.0);
    }

    #[test]
    fn thermo_dormant_gives_zero() {
        // Cycle: 10 active, 10 dormant. Tick 15 is in dormant phase.
        let income = thermo_income(1.0, 8.0, 1, 15, (10, 10));
        assert!(
            income.abs() < f32::EPSILON,
            "dormant vent should give 0, got {income}"
        );
    }

    #[test]
    fn thermo_active_gives_income() {
        // Tick 5 is in active phase (0-9 active, 10-19 dormant)
        let income = thermo_income(1.0, 8.0, 1, 5, (10, 10));
        assert!(income > 0.0, "active vent should give income");
    }

    #[test]
    fn thermo_shared_among_adjacent() {
        let alone = thermo_income(1.0, 8.0, 1, 0, (0, 0));
        let shared = thermo_income(1.0, 8.0, 4, 0, (0, 0));
        assert!(
            alone > shared,
            "alone ({alone}) should get more than shared among 4 ({shared})"
        );
    }

    #[test]
    fn thermo_cycles_back_to_active() {
        // Full cycle = 10 + 10 = 20. Tick 25 = tick 5 in second cycle → active
        let income = thermo_income(1.0, 8.0, 1, 25, (10, 10));
        assert!(income > 0.0, "should be active again in second cycle");
    }

    // ── Scavenge tests ──────────────────────────────────────────────

    #[test]
    fn scavenge_zero_with_no_gene() {
        let (income, consumed) = scavenge_income(0.0, 10.0);
        assert!(income.abs() < f32::EPSILON);
        assert!(consumed.abs() < f32::EPSILON);
    }

    #[test]
    fn scavenge_zero_on_empty_tile() {
        let (income, consumed) = scavenge_income(1.0, 0.0);
        assert!(income.abs() < f32::EPSILON);
        assert!(consumed.abs() < f32::EPSILON);
    }

    #[test]
    fn scavenge_extracts_proportional_amount() {
        let (high_income, _) = scavenge_income(0.8, 10.0);
        let (low_income, _) = scavenge_income(0.2, 10.0);
        assert!(high_income > low_income);
    }

    #[test]
    fn scavenge_consumed_does_not_exceed_available() {
        let (_, consumed) = scavenge_income(1.0, 0.5);
        assert!(
            consumed <= 0.5 + f32::EPSILON,
            "consumed {consumed} exceeds available 0.5"
        );
    }

    #[test]
    fn scavenge_income_is_fraction_of_consumed() {
        // Scavenging is lossy: cell absorbs 90% of what it removes
        let (income, consumed) = scavenge_income(0.6, 5.0);
        assert!(
            (income - consumed * SCAVENGE_EFFICIENCY).abs() < f32::EPSILON,
            "income ({income}) should be {SCAVENGE_EFFICIENCY}x consumed ({consumed})"
        );
    }

    // ── Metabolic cost tests ────────────────────────────────────────

    fn default_config() -> WorldConfig {
        WorldConfig::default()
    }

    /// Build DecodedGenes with all values set to `val`.
    fn uniform_genes(val: f32) -> DecodedGenes {
        DecodedGenes {
            values: [val; BASE_GENE_COUNT],
        }
    }

    #[test]
    fn metabolic_cost_zero_genome_matched_temp() {
        // All genes zero + perfectly matched temperature → zero cost
        let config = default_config();
        let genes = uniform_genes(0.0);
        let cost = metabolic_cost(&genes, 0, &config); // temp 0, pref 0.0 → no mismatch
        assert!(
            cost.abs() < f32::EPSILON,
            "zero genome with matched temp should cost nothing, got {cost}"
        );
    }

    #[test]
    fn metabolic_cost_superlinear() {
        // Doubling gene values should MORE than double the cost
        let config = default_config();
        let low = uniform_genes(0.3);
        let high = uniform_genes(0.6);
        let cost_low = metabolic_cost(&low, 128, &config);
        let cost_high = metabolic_cost(&high, 128, &config);
        let ratio = cost_high / cost_low;
        assert!(
            ratio > 2.0,
            "cost ratio {ratio} should be > 2.0 (superlinear)"
        );
    }

    #[test]
    fn metabolic_cost_all_max_exceeds_income() {
        // Key invariant: all genes maxed costs more than any income source
        let config = default_config();
        let maxed = uniform_genes(1.0);
        let cost = metabolic_cost(&maxed, 128, &config);
        assert!(
            cost > PHOTO_MAX_INCOME,
            "all-max cost ({cost}) should exceed max photo income ({PHOTO_MAX_INCOME})"
        );
    }

    #[test]
    fn metabolic_cost_temperature_mismatch_penalty() {
        let config = default_config();
        let mut genes = uniform_genes(0.5);
        // temperature_preference gene at index 36, set to 0.0 (prefers cold)
        genes.values[genome::TEMPERATURE_PREFERENCE] = 0.0;

        let matched = metabolic_cost(&genes, 0, &config); // cold tile, cold pref
        let mismatched = metabolic_cost(&genes, 255, &config); // hot tile, cold pref
        assert!(
            mismatched > matched,
            "mismatch ({mismatched}) should cost more than match ({matched})"
        );
    }

    // ── Venom & toxin damage tests ──────────────────────────────────

    #[test]
    fn venom_zero_damage_when_none() {
        assert!(venom_tick_damage(0, 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn venom_membrane_reduces_damage() {
        let no_membrane = venom_tick_damage(10, 0.0);
        let full_membrane = venom_tick_damage(10, 1.0);
        assert!(
            full_membrane < no_membrane,
            "membrane should reduce: no_membrane={no_membrane}, full={full_membrane}"
        );
    }

    #[test]
    fn venom_membrane_cannot_go_negative() {
        let dmg = venom_tick_damage(5, 1.0);
        assert!(dmg >= 0.0, "damage should not be negative, got {dmg}");
    }

    #[test]
    fn toxin_zero_on_clean_tile() {
        assert!(toxin_tile_damage(0.0, 0.5, 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn toxin_resistance_reduces_damage() {
        let no_resist = toxin_tile_damage(5.0, 0.0, 0.0);
        let full_resist = toxin_tile_damage(5.0, 1.0, 0.0);
        assert!(
            full_resist < no_resist,
            "resistance should reduce: none={no_resist}, full={full_resist}"
        );
    }

    #[test]
    fn toxin_membrane_also_reduces_damage() {
        let no_membrane = toxin_tile_damage(5.0, 0.0, 0.0);
        let with_membrane = toxin_tile_damage(5.0, 0.0, 1.0);
        assert!(
            with_membrane < no_membrane,
            "membrane should reduce toxin: none={no_membrane}, with={with_membrane}"
        );
    }

    #[test]
    fn toxin_both_defenses_stack() {
        let neither = toxin_tile_damage(5.0, 0.0, 0.0);
        let resist_only = toxin_tile_damage(5.0, 0.5, 0.0);
        let both = toxin_tile_damage(5.0, 0.5, 0.5);
        assert!(
            both < resist_only,
            "both defenses should be better than one"
        );
        assert!(resist_only < neither);
    }

    // ── update_energy tests ─────────────────────────────────────────

    use crate::sim::cell::Cell;
    use crate::sim::genome::{GENOME_LEN, Genome};

    fn make_test_cell(energy: f32) -> Cell {
        Cell::new(Genome::new([0u8; GENOME_LEN]), energy, (5, 5))
    }

    fn base_ctx() -> EnergyContext {
        let mut decoded = uniform_genes(0.0);
        // Default storage cap to 1.0 so energy isn't clamped to zero
        decoded.values[genome::ENERGY_STORAGE_CAP] = 1.0;
        EnergyContext {
            decoded,
            tile_sunlight: 0,
            tile_temperature: 0,
            tile_toxin: 0.0,
            tile_decay: 0.0,
            vent_income: 0.0,
        }
    }

    #[test]
    fn update_energy_photo_adds_income() {
        let config = default_config();
        let mut cell = make_test_cell(50.0);
        let mut ctx = base_ctx();
        ctx.decoded.values[genome::PHOTOSYNTHESIS_RATE] = 1.0;
        ctx.tile_sunlight = 255;

        let result = update_energy(&mut cell, &ctx, &config);
        assert!(cell.energy > 50.0, "should gain energy from photosynthesis");
        assert!(!result.died);
    }

    #[test]
    fn update_energy_dies_at_zero() {
        let config = default_config();
        let mut cell = make_test_cell(0.1);
        // High gene values → high metabolic cost → death
        let mut ctx = base_ctx();
        ctx.decoded = uniform_genes(1.0);

        let result = update_energy(&mut cell, &ctx, &config);
        assert!(
            cell.energy <= 0.0,
            "should have died, energy={}",
            cell.energy
        );
        assert!(result.died);
    }

    #[test]
    fn update_energy_capped_at_storage() {
        let config = default_config();
        let mut cell = make_test_cell(1000.0);
        let mut ctx = base_ctx();
        ctx.decoded.values[genome::PHOTOSYNTHESIS_RATE] = 1.0;
        ctx.decoded.values[genome::ENERGY_STORAGE_CAP] = 0.5;
        ctx.tile_sunlight = 255;

        update_energy(&mut cell, &ctx, &config);
        // energy_storage_cap gene is 0.5 (normalized). The max storage
        // should cap the cell's energy.
        assert!(cell.energy <= 1000.0 + PHOTO_MAX_INCOME);
    }

    #[test]
    fn update_energy_scavenge_returns_consumed() {
        let config = default_config();
        let mut cell = make_test_cell(50.0);
        let mut ctx = base_ctx();
        ctx.decoded.values[genome::SCAVENGE_ABILITY] = 0.5;
        ctx.tile_decay = 10.0;

        let result = update_energy(&mut cell, &ctx, &config);
        assert!(result.decay_consumed > 0.0, "should consume some decay");
    }

    #[test]
    fn update_energy_venom_drains() {
        let config = default_config();
        let mut cell = make_test_cell(50.0);
        cell.venom_ticks = 3;
        cell.venom_damage = 10;
        let ctx = base_ctx();

        update_energy(&mut cell, &ctx, &config);
        assert!(cell.energy < 50.0, "venom should drain energy");
        assert_eq!(cell.venom_ticks, 2, "venom ticks should decrement");
    }
}
