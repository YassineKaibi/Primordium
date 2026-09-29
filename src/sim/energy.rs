// @veridikt
// kind: module
// name: Energy
// purpose: "The per-tick energy economy: income from photosynthesis/thermosynthesis/scavenging, metabolic drain, venom/toxin damage, storage cap, and death"
// owner: "primordium-maintainers"
// because: "Energy is the single selection currency — every gene a cell expresses costs energy to run, so the balance between income channels and metabolic_cost is what selection acts on"

// Energy income (photo/thermo/scavenge), metabolic drain, starvation

use crate::config::WorldConfig;
use crate::sim::cell::Cell;
use crate::sim::genome::{self, DecodedGenes};
use crate::sim::stats::DeathCause;

// ── Storage capacity ───────────────────────────────────────────────

/// Energy a cell can hold: `energy_cap_floor` plus the gene's share of the
/// range up to `energy_cap_max`.
///
/// The floor matters. With a bare `gene * 255` most founders held less than
/// one tick of income, so their spawn energy was clipped away on tick 1 and
/// their reproduction threshold sat near zero.

// @veridikt
// purpose: "Single definition of a cell's energy storage capacity from its decoded gene and the config floor/max"
// because: "The cap bounds hoarding, clips income and sets the reproduction threshold, so all three must read one formula"
pub fn storage_cap(decoded: &DecodedGenes, config: &WorldConfig) -> f32 {
    let span = (config.energy_cap_max - config.energy_cap_floor).max(0.0);
    config.energy_cap_floor + decoded.get(genome::ENERGY_STORAGE_CAP) * span
}

// ── Lifespan ───────────────────────────────────────────────────────

/// Ticks a cell lives before dying of old age, from its `max_age` gene.

// @veridikt
// purpose: "Map the max_age gene onto an absolute lifespan between the configured min and max"
// because: "docs/spec.md gene 32 is 'tick count before natural death'; without it a cell that reaches energy equilibrium never dies, never reproduces, and its colony freezes"
pub fn lifespan_ticks(decoded: &DecodedGenes, config: &WorldConfig) -> u32 {
    let span = config
        .max_lifespan_ticks
        .saturating_sub(config.min_lifespan_ticks) as f32;
    config.min_lifespan_ticks + (decoded.get(genome::MAX_AGE) * span) as u32
}

// ── Corpse body ────────────────────────────────────────────────────

/// Structural matter a corpse leaves, whatever energy it still held:
/// `corpse_biomass`, or with `corpse_growth_ticks` that share of it the body
/// had grown to, `age / corpse_growth_ticks`, capped at the full biomass.
///
/// A body is never paid for, so its biomass is energy made at death. At 50 a
/// cell that starved young left more than the energy it cost to make, and
/// scavengers bred children that starved to feed the next brood: on one seed
/// the population went 7 249 → 95 473 between t=7 000 and t=10 000 (handover
/// step 20). A body that grows with age leaves a newborn little and an old
/// cell the whole baseline.

// @veridikt
// purpose: "Biomass a corpse leaves on top of its remaining energy, grown with the cell's age when corpse_growth_ticks is set"
// because: "Biomass is energy that appears at death; a full body for a newborn made short lives a net energy source that evolution found (a starvation pump), while a body that grows with age keeps the full baseline for old corpses"
pub fn corpse_body(age: u32, config: &WorldConfig) -> f32 {
    if config.corpse_growth_ticks == 0 {
        return config.corpse_biomass;
    }
    let grown = (age as f32 / config.corpse_growth_ticks as f32).min(1.0);
    config.corpse_biomass * grown
}

// ── Corpse persistence ─────────────────────────────────────────────

/// Per-tick fade fraction for the decay matter one corpse leaves behind.
///
/// `docs/spec.md` gene 30, `decay_rate`: "how long the cell's corpse
/// persists as scavengeable decay matter". The gene scales the world's
/// `decay_rate` between `corpse_decay_scale_min` and
/// `corpse_decay_scale_max`, so a low gene is a tough body that stays
/// edible far longer than the world average.

// @veridikt
// purpose: "Map a cell's decay_rate gene onto the per-tick fade of the decay matter its corpse leaves"
// because: "Corpses are point sources that a scavenger has to walk to; how long one lasts decides whether the scavenger niche has any supply at all, and spec.md makes that a property of the dead cell rather than of the world"
pub fn corpse_decay_fade(decoded: &DecodedGenes, config: &WorldConfig) -> f32 {
    let span = config.corpse_decay_scale_max - config.corpse_decay_scale_min;
    let scale = config.corpse_decay_scale_min + decoded.get(genome::DECAY_RATE) * span;
    (config.decay_rate * scale).max(0.0)
}

// ── Photosynthesis ─────────────────────────────────────────────────

/// Calculate photosynthesis energy income for a cell.
///
/// `effective_rate`: decoded photosynthesis_rate gene (0.0..1.0)
/// `tile_sunlight`: sunlight value on the cell's tile (0..255)

// @veridikt
// purpose: "Photosynthesis income: scale the capped max income by the cell's effective rate and local sunlight"
// because: "config.photo_max_income is tuned below a generalist's metabolic cost so photosynthesis only pays off for lean, specialized genomes"
pub fn photo_income(effective_rate: f32, tile_sunlight: u8, config: &WorldConfig) -> f32 {
    let sunlight_norm = tile_sunlight as f32 / 255.0;
    config.photo_max_income * effective_rate * sunlight_norm
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

// @veridikt
// purpose: "Thermal-vent income for cells adjacent to a vent, shared equally among all adjacent cells and gated by the vent's active/dormant cycle"
// because: "Vent output is divided by adjacent_count, so crowding a vent dilutes everyone's share — this caps colony density at vents the way shade caps it at the surface"
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

// @veridikt
// purpose: "Scavenge income from decay matter on the tile, returning both the energy gained and the decay to subtract"
// because: "Scavenging is lossy (config.scavenge_efficiency) and bounded by available decay, so it is an energy sink that recycles dead biomass rather than free energy"
pub fn scavenge_income(
    effective_ability: f32,
    tile_decay: f32,
    config: &WorldConfig,
) -> (f32, f32) {
    let decay_consumed = (effective_ability * tile_decay)
        .min(tile_decay)
        .min(config.max_scavenge_per_tick);
    (decay_consumed * config.scavenge_efficiency, decay_consumed)
}

// ── Metabolic cost ─────────────────────────────────────────────────

/// Calculate total metabolic energy drain per tick.
///
/// Each gene's cost = `gene_value ^ exponent`, summed across all 46 genes,
/// or across all but `genome::RAW_READ_GENES` with
/// `raw_genes_outside_expression`: nothing reads their decoded values, so
/// they buy nothing to pay for.
/// The superlinear exponent (default 1.5) makes high gene values
/// disproportionately expensive — this is the core anti-supercell mechanic.
///
/// An additional penalty is applied for temperature mismatch between the
/// cell's `temperature_preference` gene and the local tile temperature.
///
/// `decoded`: effective gene values after all expression constraints
/// `tile_temperature`: local tile temperature (0..255)
/// `config`: world config (for exponent and temperature mismatch cost)

// @veridikt
// purpose: "Total per-tick energy drain: sum of each gene value raised to a superlinear exponent, plus a temperature-mismatch penalty"
// because: "The superlinear exponent (default 1.5) is the core anti-supercell mechanic — maxing many genes costs disproportionately more than any income channel can supply, so generalists starve"
// depends_on: Genome.RAW_READ_GENES
pub fn metabolic_cost(decoded: &DecodedGenes, tile_temperature: u8, config: &WorldConfig) -> f32 {
    let skip_raw = config.raw_genes_outside_expression;
    let mut base_cost = 0.0_f32;
    for (i, &value) in decoded.values.iter().enumerate() {
        if skip_raw && genome::IS_RAW_READ[i] {
            continue;
        }
        base_cost += value.powf(config.metabolic_cost_exponent);
    }
    base_cost *= config.metabolic_cost_scale;

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

// ── Adaptation ─────────────────────────────────────────────────────

/// A cell's effective temperature preference: its inherited gene plus the
/// acclimation it has built up during its own life.
pub fn acclimated_preference(decoded: &DecodedGenes, cell: &Cell) -> f32 {
    (decoded.get(genome::TEMPERATURE_PREFERENCE) + cell.temp_acclimation).clamp(0.0, 1.0)
}

/// Move a cell's temperature acclimation one tick toward the temperature it is
/// actually living at.
///
/// `docs/spec.md` gene 38, `adaptation_rate`: "speed of within-lifetime
/// epigenetic-like modifier shifts. Not inherited." The spec names no target,
/// and temperature is the one trait with an obvious one: a cell that sits in
/// the wrong water pays `temperature_mismatch_cost` every tick, and
/// acclimation lets it close that gap over its life at a rate its genome
/// sets. The shift lives on the cell, not the genome, and every newborn starts
/// at zero, so none of it is inherited.

// @veridikt
// purpose: "Close part of the gap between a cell's effective temperature preference and its tile's temperature each tick, at a rate set by its adaptation_rate gene"
// because: "spec.md gene 38 is a within-lifetime, non-inherited modifier shift; temperature mismatch is the cost with a clear local target, so acclimation is where that shift has something to do"
pub fn acclimate(
    cell: &mut Cell,
    inherited_preference: f32,
    adaptation_rate: f32,
    tile_temperature: u8,
    config: &WorldConfig,
) {
    let rate = adaptation_rate * config.max_adaptation_rate;
    if rate <= 0.0 {
        return;
    }
    let target = tile_temperature as f32 / 255.0;
    let current = (inherited_preference + cell.temp_acclimation).clamp(0.0, 1.0);
    cell.temp_acclimation += (target - current) * rate;
}

// ── Dormancy ───────────────────────────────────────────────────────

/// Is this cell dormant, and what does its metabolism cost while it is?
///
/// `docs/spec.md` gene 34, `dormancy_trigger`: "energy threshold below which
/// the cell enters dormancy phase"; gene 35, `dormancy_cost`: "energy drain
/// rate while in dormancy (lower = better hibernation)". Neither gene was
/// read anywhere, so a starving cell had no way to ride out a famine and
/// every lineage's runway was exactly `storage_cap / metabolic_cost`.
///
/// Returns the multiplier to apply to the cell's metabolic cost: 1.0 when
/// awake, `dormancy_cost` when dormant. A dormant cell still pays its
/// temperature mismatch and still takes venom and toxin damage — it is
/// slowed, not sealed.

// @veridikt
// purpose: "Decide whether a cell is below its dormancy_trigger and return the metabolic multiplier that its dormancy_cost gene buys"
// because: "Without it there is no way to survive a gap in the food supply, and the scavenger niche in particular cannot exist until the first corpses appear ~1300 ticks into a run"
pub fn dormancy_multiplier(
    decoded: &DecodedGenes,
    energy_fraction: f32,
    config: &WorldConfig,
) -> f32 {
    let trigger = decoded.get(genome::DORMANCY_TRIGGER) * config.max_dormancy_trigger;
    if trigger <= 0.0 || energy_fraction >= trigger {
        return 1.0;
    }
    decoded
        .get(genome::DORMANCY_COST)
        .clamp(config.min_dormancy_cost, 1.0)
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
    /// Income by channel this tick: photosynthesis, vent, scavenging.
    pub photo: f32,
    pub thermo: f32,
    pub scavenge: f32,
    /// Metabolic cost actually paid (after any dormancy discount).
    pub metabolism: f32,
    pub venom: f32,
    pub toxin: f32,
    /// Energy above the storage cap, destroyed by the clamp.
    pub cap_waste: f32,
    pub dormant: bool,
    /// Which drain took the cell to zero, when it died.
    pub cause: Option<DeathCause>,
    /// Energy the cell still held when senescence zeroed it; 0 for any cell
    /// that did not die of old age this tick.
    pub senesced_energy: f32,
}

/// Apply all energy income and costs to a cell for one tick.
/// Mutates `cell.energy` in place. Returns info the caller needs.

// @veridikt
// purpose: "Settle one cell's energy for the tick: add all income, subtract metabolism + venom + toxin, clamp to storage cap, and flag death at <=0 (reporting what a cell dying of old age still held)"
// because: "Order is income, then costs, then cap, then death-check — so a cell that earns a lot is still capped, and the storage cap (ENERGY_STORAGE_CAP gene) bounds hoarding"
// assumes: "ctx.decoded already has phase modifiers applied by the caller, and ctx.vent_income was pre-computed from vent adjacency"
pub fn update_energy(cell: &mut Cell, ctx: &EnergyContext, config: &WorldConfig) -> EnergyResult {
    let photosynthesis_income = photo_income(
        ctx.decoded.get(genome::PHOTOSYNTHESIS_RATE),
        ctx.tile_sunlight,
        config,
    );
    let (scavenge_income, decay_consumed) = scavenge_income(
        ctx.decoded.get(genome::SCAVENGE_ABILITY),
        ctx.tile_decay,
        config,
    );

    cell.energy += photosynthesis_income + ctx.vent_income + scavenge_income;

    // Dormancy is judged on the energy the cell holds *before* this tick's
    // drain, so a cell that just fed its way back above the trigger wakes up
    // in the same tick.
    let max_energy = storage_cap(&ctx.decoded, config);
    let energy_fraction = if max_energy > 0.0 {
        cell.energy / max_energy
    } else {
        0.0
    };
    let dormancy = dormancy_multiplier(&ctx.decoded, energy_fraction, config);
    let metabolism = metabolic_cost(&ctx.decoded, ctx.tile_temperature, config) * dormancy;
    cell.energy -= metabolism;
    // The first drain to reach zero is the cause; later ones only deepen it.
    let mut cause = (cell.energy <= 0.0).then_some(DeathCause::Starvation);

    let mut venom = 0.0;
    if cell.venom_ticks > 0 {
        venom = venom_tick_damage(cell.venom_damage, ctx.decoded.get(genome::MEMBRANE));
        cell.energy -= venom;
        cell.venom_ticks -= 1;
        if cause.is_none() && cell.energy <= 0.0 {
            cause = Some(DeathCause::Venom);
        }
    }

    let toxin = toxin_tile_damage(
        ctx.tile_toxin,
        ctx.decoded.get(genome::TOXIN_RESISTANCE),
        ctx.decoded.get(genome::MEMBRANE),
    );
    cell.energy -= toxin;
    if cause.is_none() && cell.energy <= 0.0 {
        cause = Some(DeathCause::Toxin);
    }

    let cap_waste = (cell.energy - max_energy).max(0.0);
    cell.energy = cell.energy.min(max_energy);

    // Senescence: old age kills regardless of how well fed the cell is.
    // What it still held is reported, for `corpses_keep_energy`: liveness is
    // `energy > 0`, so the corpse itself cannot carry it to cleanup.
    let mut senesced_energy = 0.0;
    if cell.age >= lifespan_ticks(&ctx.decoded, config) {
        senesced_energy = cell.energy.max(0.0);
        cell.energy = 0.0;
        cause = cause.or(Some(DeathCause::OldAge));
    }

    let died = cell.energy <= 0.0;

    EnergyResult {
        decay_consumed,
        died,
        photo: photosynthesis_income,
        thermo: ctx.vent_income,
        scavenge: scavenge_income,
        metabolism,
        venom,
        toxin,
        cap_waste,
        dormant: dormancy < 1.0,
        cause: if died { cause } else { None },
        senesced_energy,
    }
}

// ── Tests ───────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// spec.md gene 38, adaptation_rate: "speed of within-lifetime
    /// epigenetic-like modifier shifts. Not inherited." It was the one gene
    /// nothing read.
    #[test]
    fn a_cell_acclimates_to_its_water_at_its_own_rate_and_passes_none_of_it_on() {
        let config = WorldConfig::default();
        let tile_temperature = 230u8; // warm water
        let mut data = [0u8; genome::GENOME_LEN];
        data[genome::TEMPERATURE_PREFERENCE] = 40; // a cold-water genome
        let mismatch_after = |adaptation: u8, ticks: u32| {
            let mut d = data;
            d[genome::ADAPTATION_RATE] = adaptation;
            let g = Genome::new(d);
            let decoded = g.decode(&config);
            let mut cell = Cell::new(g, 100.0, (0, 0));
            for _ in 0..ticks {
                acclimate(
                    &mut cell,
                    decoded.get(genome::TEMPERATURE_PREFERENCE),
                    decoded.get(genome::ADAPTATION_RATE),
                    tile_temperature,
                    &config,
                );
            }
            ((tile_temperature as f32 / 255.0) - acclimated_preference(&decoded, &cell)).abs()
        };
        let rigid = mismatch_after(0, 500);
        let plastic = mismatch_after(255, 500);
        assert!(
            plastic < rigid * 0.5,
            "acclimation closed {rigid:.3} only to {plastic:.3}"
        );
        // Nothing on the genome changed, and a newborn starts from zero.
        assert_eq!(
            Cell::new(Genome::new(data), 1.0, (0, 0)).temp_acclimation,
            0.0
        );
    }

    /// spec.md genes 34/35. Neither was read anywhere, so a starving cell
    /// had no way to ride out a gap in its food supply.
    /// A corpse has to last long enough to be a food source. Uncapped, a
    /// maxed scavenger strips the whole tile on the tick it arrives, so
    /// `corpse_biomass` 25 is one 22-energy meal against an upkeep of ~1.2,
    /// and the niche only pays for a cell that can reach a fresh body every
    /// tick — which, at a 1-tile-per-tick movement ceiling, none can.
    #[test]
    fn one_corpse_feeds_a_scavenger_for_several_ticks() {
        let config = WorldConfig::default();
        let mut decay = config.corpse_biomass;
        let mut ticks = 0;
        while decay > 0.01 && ticks < 1000 {
            let (income, consumed) = scavenge_income(1.0, decay, &config);
            assert!(consumed <= config.max_scavenge_per_tick + f32::EPSILON);
            assert!(income > 0.0);
            decay -= consumed;
            ticks += 1;
        }
        assert!(
            ticks >= 8,
            "one corpse was gone in {ticks} tick(s); it is a meal, not a supply"
        );
    }

    #[test]
    fn a_dormant_cell_burns_less_and_outlives_one_that_cannot_hibernate() {
        let config = WorldConfig::default();
        let mut data = [40u8; genome::GENOME_LEN];
        data[genome::DORMANCY_TRIGGER] = 0;
        let awake = Genome::new(data).decode(&config);
        data[genome::DORMANCY_TRIGGER] = 200; // shuts down below 78% full
        data[genome::DORMANCY_COST] = 10; // and barely ticks over while down
        let sleeper = Genome::new(data).decode(&config);

        assert_eq!(dormancy_multiplier(&awake, 0.05, &config), 1.0);
        let slowed = dormancy_multiplier(&sleeper, 0.05, &config);
        assert!(
            slowed < 1.0,
            "a cell below its trigger still paid {slowed} of its upkeep"
        );
        // Above the trigger it is awake again.
        assert_eq!(dormancy_multiplier(&sleeper, 0.99, &config), 1.0);

        // And it really does last longer on the same starting energy.
        let ticks_to_starve = |decoded: &DecodedGenes| {
            let mut cell = Cell::new(Genome::new(data), 40.0, (0, 0));
            let ctx = EnergyContext {
                decoded: decoded.clone(),
                tile_sunlight: 0,
                tile_temperature: 128,
                tile_toxin: 0.0,
                tile_decay: 0.0,
                vent_income: 0.0,
            };
            (1..100_000)
                .find(|_| update_energy(&mut cell, &ctx, &config).died)
                .unwrap_or(100_000)
        };
        // `min_dormancy_cost` caps the benefit at 1/0.25 = 4x on purpose:
        // an arbitrarily cheap hibernator neither reproduces nor dies, so it
        // just occupies a tile forever.
        assert!(
            ticks_to_starve(&sleeper) > ticks_to_starve(&awake) * 3 / 2,
            "hibernating bought {} ticks against {} awake",
            ticks_to_starve(&sleeper),
            ticks_to_starve(&awake)
        );
    }

    use crate::sim::genome::BASE_GENE_COUNT;

    // ── Photosynthesis tests ────────────────────────────────────────

    #[test]
    fn photo_zero_in_dark_tile() {
        // No sunlight → no income regardless of gene value
        assert!((photo_income(1.0, 0, &default_config())).abs() < f32::EPSILON);
    }

    #[test]
    fn photo_zero_with_no_gene() {
        // Gene is 0 → no income regardless of sunlight
        assert!((photo_income(0.0, 255, &default_config())).abs() < f32::EPSILON);
    }

    #[test]
    fn photo_proportional_to_sunlight() {
        let bright = photo_income(0.5, 200, &default_config());
        let dim = photo_income(0.5, 50, &default_config());
        assert!(bright > dim, "bright ({bright}) should exceed dim ({dim})");
    }

    #[test]
    fn photo_proportional_to_rate() {
        let high = photo_income(0.8, 128, &default_config());
        let low = photo_income(0.2, 128, &default_config());
        assert!(
            high > low,
            "high rate ({high}) should exceed low rate ({low})"
        );
    }

    #[test]
    fn photo_max_gives_reasonable_value() {
        let income = photo_income(1.0, 255, &default_config());
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
        let (income, consumed) = scavenge_income(0.0, 10.0, &default_config());
        assert!(income.abs() < f32::EPSILON);
        assert!(consumed.abs() < f32::EPSILON);
    }

    #[test]
    fn scavenge_zero_on_empty_tile() {
        let (income, consumed) = scavenge_income(1.0, 0.0, &default_config());
        assert!(income.abs() < f32::EPSILON);
        assert!(consumed.abs() < f32::EPSILON);
    }

    #[test]
    fn scavenge_extracts_proportional_amount() {
        let (high_income, _) = scavenge_income(0.8, 10.0, &default_config());
        let (low_income, _) = scavenge_income(0.2, 10.0, &default_config());
        assert!(high_income > low_income);
    }

    #[test]
    fn scavenge_consumed_does_not_exceed_available() {
        let (_, consumed) = scavenge_income(1.0, 0.5, &default_config());
        assert!(
            consumed <= 0.5 + f32::EPSILON,
            "consumed {consumed} exceeds available 0.5"
        );
    }

    #[test]
    fn scavenge_income_is_fraction_of_consumed() {
        // Scavenging is lossy: cell absorbs 90% of what it removes
        let (income, consumed) = scavenge_income(0.6, 5.0, &default_config());
        assert!(
            (income - consumed * default_config().scavenge_efficiency).abs() < f32::EPSILON,
            "income ({income}) should be {}x consumed ({consumed})",
            default_config().scavenge_efficiency
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
    fn old_age_kills_a_well_fed_cell() {
        // Without senescence a cell at energy equilibrium lives forever,
        // never reproduces, and its colony freezes in place.
        let config = default_config();
        let mut genes = uniform_genes(0.03);
        genes.values[genome::MAX_AGE] = 0.0; // shortest lifespan
        genes.values[genome::ENERGY_STORAGE_CAP] = 1.0;

        let ctx = EnergyContext {
            decoded: genes,
            tile_sunlight: 255,
            tile_temperature: 128,
            tile_toxin: 0.0,
            tile_decay: 0.0,
            vent_income: 0.0,
        };

        let mut young = Cell::new(Genome::new([8u8; genome::GENOME_LEN]), 200.0, (0, 0));
        young.age = config.min_lifespan_ticks - 1;
        assert!(!update_energy(&mut young, &ctx, &config).died);

        let mut old = Cell::new(Genome::new([8u8; genome::GENOME_LEN]), 200.0, (0, 0));
        old.age = config.min_lifespan_ticks;
        assert!(
            update_energy(&mut old, &ctx, &config).died,
            "old age must kill"
        );
    }

    /// The first drain that reaches zero is the cause of death, and a cell
    /// that starves at the end of its lifespan died of starvation, not age.
    #[test]
    fn a_death_is_attributed_to_the_drain_that_caused_it() {
        let config = default_config();
        let mut genes = uniform_genes(0.03);
        genes.values[genome::MAX_AGE] = 0.0;
        // No dormancy, so a low-energy cell pays its full upkeep.
        genes.values[genome::DORMANCY_TRIGGER] = 0.0;
        let ctx = EnergyContext {
            decoded: genes,
            tile_sunlight: 0,
            tile_temperature: 128,
            tile_toxin: 0.0,
            tile_decay: 0.0,
            vent_income: 0.0,
        };
        let genome = || Genome::new([8u8; genome::GENOME_LEN]);

        let mut fed = Cell::new(genome(), 100.0, (0, 0));
        let r = update_energy(&mut fed, &ctx, &config);
        assert!(!r.died && r.cause.is_none());
        let upkeep = r.metabolism;
        assert!(upkeep > 0.0);

        let mut starving = Cell::new(genome(), upkeep * 0.5, (0, 0));
        let r = update_energy(&mut starving, &ctx, &config);
        assert_eq!(r.cause, Some(DeathCause::Starvation));

        // Survives its upkeep by one unit, then venom finishes it.
        let mut poisoned = Cell::new(genome(), upkeep + 1.0, (0, 0));
        poisoned.venom_ticks = 3;
        poisoned.venom_damage = 100;
        let r = update_energy(&mut poisoned, &ctx, &config);
        assert_eq!(r.cause, Some(DeathCause::Venom));

        let mut old = Cell::new(genome(), 100.0, (0, 0));
        old.age = config.min_lifespan_ticks;
        assert_eq!(
            update_energy(&mut old, &ctx, &config).cause,
            Some(DeathCause::OldAge)
        );

        let mut old_and_starving = Cell::new(genome(), upkeep * 0.5, (0, 0));
        old_and_starving.age = config.min_lifespan_ticks;
        assert_eq!(
            update_energy(&mut old_and_starving, &ctx, &config).cause,
            Some(DeathCause::Starvation)
        );
    }

    #[test]
    fn lifespan_scales_with_the_gene() {
        let config = default_config();
        let short = lifespan_ticks(&uniform_genes(0.0), &config);
        let long = lifespan_ticks(&uniform_genes(1.0), &config);
        assert_eq!(short, config.min_lifespan_ticks);
        assert_eq!(long, config.max_lifespan_ticks);
    }

    #[test]
    fn storage_cap_has_a_floor_and_a_ceiling() {
        let config = default_config();
        let empty = uniform_genes(0.0);
        let full = uniform_genes(1.0);
        assert_eq!(storage_cap(&empty, &config), config.energy_cap_floor);
        assert_eq!(storage_cap(&full, &config), config.energy_cap_max);
        // A cell that can hold less than one tick of income cannot live.
        assert!(storage_cap(&empty, &config) > config.photo_max_income);
    }

    #[test]
    fn lean_specialist_earns_more_than_it_spends() {
        // The feasibility floor of the whole economy: a cell expressing one
        // acquisition gene and little else must profit in full sunlight.
        let config = default_config();
        let mut genes = uniform_genes(0.03);
        genes.values[genome::PHOTOSYNTHESIS_RATE] = 1.0;
        genes.values[genome::ENERGY_STORAGE_CAP] = 0.8;
        genes.values[genome::TEMPERATURE_PREFERENCE] = 0.5;

        let income = photo_income(1.0, 255, &config);
        let cost = metabolic_cost(&genes, 128, &config);
        assert!(
            income > cost * 1.5,
            "specialist income {income} must clear upkeep {cost} with room to grow"
        );
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
            cost > config.photo_max_income,
            "all-max cost ({cost}) should exceed max photo income ({})",
            config.photo_max_income
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
        assert!(cell.energy <= 1000.0 + config.photo_max_income);
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
