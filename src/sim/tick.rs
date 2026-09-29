// @veridikt
// kind: module
// name: Tick
// purpose: "The per-tick orchestrator: runs the six simulation phases in a fixed order and manages the double-buffer swap"
// owner: "primordium-maintainers"
// because: "Phase order is load-bearing — environment first, then sense/decide against a frozen grid, then resolve actions into next, then energy, then cleanup, then swap — so the tick stays deterministic and order-independent"
// depends_on: Diffusion, World, Actions, Phase, Energy, Genome, Stats

// Tick orchestration: calls each phase in order, manages buffer swaps

use rand_chacha::ChaCha8Rng;

use crate::config::WorldConfig;
use crate::sim::actions::{self, Action};
use crate::sim::diffusion;
use crate::sim::energy::{self, EnergyContext};
use crate::sim::genome::{self, DecodeCache};
use crate::sim::phase::{self, PhaseInput};
use crate::sim::stats::{self, DeathCause, TickStats};
use crate::sim::world::World;

/// Execute one full simulation tick.
///
/// Phases run in strict order:
/// 1. Diffusion & environment (diffuse fields, recompute sunlight,
///    seed next with the environment fields)
/// 2. Sensing + phase evaluation + decision
/// 3. Action resolution
/// 4. Energy update
/// 5. Cleanup (dead → decay, toxin generation, pheromone emission)
/// 6. Bookkeeping (swap buffers, increment tick)

// @veridikt
// purpose: "Advance the whole world exactly one tick by invoking each phase in the fixed pipeline order"
// triggers: Diffusion.run_diffusion_phase, World.update_sunlight, World.prepare_next, Actions.resolve_all, World.swap_buffers
// because: "Sunlight is recomputed after diffusion (shade depends on the freshly diffused fields) but before sensing, so cells decide on up-to-date light; prepare_next then copies the environment into next, where the energy and cleanup phases read and write it"
pub fn run_tick(world: &mut World, config: &WorldConfig, rng: &mut ChaCha8Rng) {
    let mut cache = DecodeCache::default();
    run_tick_cached(world, config, rng, &mut cache);
}

/// `run_tick`, reusing a caller-owned decode cache across ticks so its
/// allocation is not rebuilt every tick. The cache is cleared on entry, so
/// this is behaviourally identical to `run_tick`.

// @veridikt
// purpose: "One tick, threading a per-tick decode cache through every phase that needs decoded genes; resets World.stats first and times each phase into it"
// because: "decode re-ranks all 46 genes on every call and is reached several times per cell per tick; the cache is cleared at the top of each tick so nothing can go stale across the buffer swap"
pub fn run_tick_cached(
    world: &mut World,
    config: &WorldConfig,
    rng: &mut ChaCha8Rng,
    cache: &mut DecodeCache,
) {
    cache.clear();
    world.stats = TickStats::default();
    // Wall time per phase, for profiling. Observational only.
    let mut clock = std::time::Instant::now();
    let mut lap = |world: &mut World, slot: usize| {
        world.stats.phase_ns[slot] = clock.elapsed().as_nanos() as u64;
        clock = std::time::Instant::now();
    };

    // Phase 1: Diffusion & environment
    diffusion::run_diffusion_phase(world, config);
    lap(world, 0);
    world.update_sunlight(
        config.sunlight_gradient_strength,
        config.cell_light_absorption,
    );
    lap(world, 1);
    world.prepare_next();
    lap(world, 2);

    // Phase 2: Sensing + phase evaluation + decision
    let actions = phase_sense_decide(world, config, rng, cache);
    lap(world, 3);

    // Phase 3: Action resolution
    actions::resolve_all(&actions, world, config, rng, cache);
    lap(world, 4);

    // Phase 4: Energy update
    phase_energy_update(world, config, cache);
    lap(world, 5);

    // Phase 5: Cleanup
    phase_cleanup(world, config, cache);
    lap(world, 6);

    // Phase 6: Bookkeeping
    world.swap_buffers();
    world.tick += 1;
}

/// Phase 2: For each living cell, sense → evaluate phase → decide action.

// @veridikt
// purpose: "Phase 2 — for every live cell, sense the neighborhood, evaluate phase transitions, apply phase modifiers, and record the chosen action"
// triggers: Actions.sense_cached, Phase.evaluate_phase, Phase.apply_phase_modifiers, Actions.decide, Genome.decode
// because: "Decisions are buffered into a Vec and applied later in resolve_all, so every cell decides against the same frozen `current` grid rather than seeing each other's mid-tick moves"
fn phase_sense_decide(
    world: &mut World,
    config: &WorldConfig,
    rng: &mut ChaCha8Rng,
    cache: &mut DecodeCache,
) -> Vec<(u32, Action)> {
    let cell_ids = world.cell_ids();
    let mut action_buffer: Vec<(u32, Action)> = Vec::with_capacity(cell_ids.len());

    for cell_id in cell_ids {
        let cell = world.get_cell(cell_id);
        let mut decoded = cache.get(cell_id, &cell.genome, config).clone();

        // Sense
        let sense = actions::sense_cached(cell, &decoded, world, config, cache);

        // Build PhaseInput
        let energy_cap = energy::storage_cap(&decoded, config);
        let energy_fraction = if energy_cap > 0.0 {
            cell.energy / energy_cap
        } else {
            0.0
        };
        let ticks_since_damage = if cell.last_damage_tick == 0 {
            u32::MAX
        } else {
            (world.tick as u32).saturating_sub(cell.last_damage_tick)
        };

        let phase_input = PhaseInput {
            energy_fraction,
            threat_count: sense.threat_count,
            kin_count: sense.kin_count,
            age: cell.age,
            food_nearby: sense.food_nearby,
            neighbor_count: sense.neighbor_count,
            ticks_since_damage,
            sense_radius: (decoded.get(genome::SENSE_RADIUS) * 4.0).ceil() as u32,
            maturity_threshold: actions::mapped_maturity_age(&decoded, config),
            memory_length: (decoded.get(genome::MEMORY_LENGTH) * 255.0) as u32,
        };

        // Evaluate phase transition
        let new_phase = phase::evaluate_phase(&cell.genome, &phase_input, cell.active_phase);

        // Update cell phase state, and age the directional memory by one
        // tick so memory_length is a duration and not a standing weight.
        let cell_mut = world.get_cell_mut(cell_id);
        actions::age_memory(cell_mut, &decoded);
        if new_phase != cell_mut.active_phase {
            cell_mut.active_phase = new_phase;
            cell_mut.phase_ticks = 0;
            world.stats.phase_transitions += 1;
        } else {
            cell_mut.phase_ticks = cell_mut.phase_ticks.saturating_add(1);
        }

        // Apply phase modifiers — re-borrow cell immutably
        let cell = world.get_cell(cell_id);
        phase::apply_phase_modifiers(&mut decoded, &cell.genome, new_phase);

        // Decide action
        let action = actions::decide(cell, &decoded, &sense, config, rng);
        world.stats.actions[action.priority_index()] += 1;
        action_buffer.push((cell_id, action));
    }

    action_buffer
}

/// Phase 4: Energy update for all living cells in the next grid.

// @veridikt
// purpose: "Phase 4 — compute vent income per cell, then settle every live cell's energy against its tile, consuming scavenged decay and, with corpses_keep_energy, laying down what an old cell dies holding"
// triggers: Energy.thermo_income, Energy.update_energy, Energy.corpse_decay_fade, World.deposit_decay, Phase.apply_phase_modifiers, Genome.decode, Tick.record_energy
// because: "Vent income is computed first across all vents so the per-cell share reflects the full adjacent crowd before any cell's energy is updated"
fn phase_energy_update(world: &mut World, config: &WorldConfig, cache: &mut DecodeCache) {
    let height = world.height;
    let tick = world.tick;

    // Pre-compute vent income: for each vent, find adjacent cells in
    // the next grid, compute per-cell share of thermo income.
    let mut vent_income_map: Vec<(u32, f32)> = Vec::new();
    for &vx in &world.vent_positions.clone() {
        // Vent sits at bottom row (y = height - 1)
        let vy = height as i32 - 1;
        let mut adjacent_cells: Vec<u32> = Vec::new();

        // The zone reaches `vent_radius` tiles around the vent, toroidal in
        // x along the bottom edge but bounded in y: see
        // `World::is_in_vent_zone`. Every cell standing in it divides the
        // vent's output, whether or not it can use thermosynthesis.
        let r = config.vent_radius as i32;
        for dy in -r..=0 {
            for dx in -r..=r {
                let (wx, wy) = world.wrap(vx as i32 + dx, vy + dy);
                let tile = world.next_tile(wx, wy);
                if tile.cell_id != 0 {
                    adjacent_cells.push(tile.cell_id);
                }
            }
        }

        let adjacent_count = adjacent_cells.len() as u32;
        for &cid in &adjacent_cells {
            let cell = world.get_cell(cid);
            let mut decoded = cache.get(cid, &cell.genome, config).clone();
            phase::apply_phase_modifiers(&mut decoded, &cell.genome, cell.active_phase);
            let income = energy::thermo_income(
                decoded.get(genome::THERMOSYNTHESIS_RATE),
                config.vent_output,
                adjacent_count,
                tick,
                config.vent_cycle,
            );
            vent_income_map.push((cid, income));
        }
    }

    // Build a lookup from cell_id → total vent income
    let mut vent_lookup: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    for (cid, income) in vent_income_map {
        *vent_lookup.entry(cid).or_insert(0.0) += income;
    }

    // Living cells on the next grid, in tile order. `placed_in_next` walks
    // the cell pool rather than all width*height tiles.
    let live_cells: Vec<(u32, usize)> = world
        .placed_in_next()
        .into_iter()
        .filter(|&(id, _)| world.get_cell(id).is_alive())
        .collect();

    for (cell_id, tile_idx) in live_cells {
        let tile = world.next_grid()[tile_idx];
        let cell = world.get_cell(cell_id);
        let mut decoded = cache.get(cell_id, &cell.genome, config).clone();
        phase::apply_phase_modifiers(&mut decoded, &cell.genome, cell.active_phase);
        // The mismatch cost is paid against the preference the cell has
        // acclimated to, not only the one it inherited (spec.md gene 38).
        let inherited_preference = decoded.get(genome::TEMPERATURE_PREFERENCE);
        let adaptation_rate = decoded.get(genome::ADAPTATION_RATE);
        decoded.values[genome::TEMPERATURE_PREFERENCE] =
            energy::acclimated_preference(&decoded, cell);

        let ctx = EnergyContext {
            decoded,
            tile_sunlight: tile.sunlight,
            tile_temperature: tile.temperature,
            tile_toxin: tile.toxin,
            tile_decay: tile.decay_energy,
            vent_income: vent_lookup.get(&cell_id).copied().unwrap_or(0.0),
        };

        let cell_mut = world.get_cell_mut(cell_id);
        let result = energy::update_energy(cell_mut, &ctx, config);
        record_energy(world, cell_id, &result);
        // Acclimate against the genome's own preference, not the already
        // acclimated one written into `ctx.decoded` above.
        energy::acclimate(
            world.get_cell_mut(cell_id),
            inherited_preference,
            adaptation_rate,
            tile.temperature,
            config,
        );

        if result.decay_consumed > 0.0 {
            world.next_grid_mut()[tile_idx].decay_energy -= result.decay_consumed;
            if world.next_grid_mut()[tile_idx].decay_energy < 0.0 {
                world.next_grid_mut()[tile_idx].decay_energy = 0.0;
            }
        }

        // An old corpse leaves `corpse_energy_fraction` of what it held, like
        // any other (spec.md, Decay Matter). Senescence has already zeroed the
        // cell, so it is laid down here; cleanup adds the `corpse_biomass`.
        if config.corpses_keep_energy && result.senesced_energy > 0.0 {
            let amount = result.senesced_energy * config.corpse_energy_fraction;
            let cell = world.get_cell(cell_id);
            let fade = energy::corpse_decay_fade(cache.get(cell_id, &cell.genome, config), config);
            world.deposit_decay(tile_idx, amount, fade);
            world.stats.decay_deposited += amount as f64;
        }
    }
}

/// Count one cell's energy settlement into the tick stats and its record.

// @veridikt
// purpose: "Add one cell's energy flows to the tick stats and its lifetime record, and set (or clear) its death cause"
// triggers: World.record_mut
// because: "Only living cells reach the energy phase, so writing the cause unconditionally also clears a label left by a blow or birth earlier in the tick on a cell that was then revived"
fn record_energy(world: &mut World, cell_id: u32, result: &energy::EnergyResult) {
    let s = &mut world.stats;
    s.energy_samples += 1;
    s.income[stats::PHOTO] += result.photo as f64;
    s.income[stats::THERMO] += result.thermo as f64;
    s.income[stats::SCAVENGE] += result.scavenge as f64;
    s.metabolism += result.metabolism as f64;
    s.venom += result.venom as f64;
    s.toxin += result.toxin as f64;
    s.cap_waste += result.cap_waste as f64;
    if result.dormant {
        s.dormant += 1;
    }
    let r = world.record_mut(cell_id);
    r.income[stats::PHOTO] += result.photo;
    r.income[stats::THERMO] += result.thermo;
    r.income[stats::SCAVENGE] += result.scavenge;
    r.upkeep += result.metabolism;
    // Every cell that reaches the energy phase is alive, so this also clears
    // a label left by an earlier blow or birth this tick on a cell that was
    // revived before the phase ran.
    r.death = result.cause;
}

/// Phase 5: Cleanup — process dead cells, generate toxin, update living cells.

// @veridikt
// purpose: "Phase 5 — turn dead cells into decay matter, spawn toxin at death clusters, and age/cooldown/emit-pheromone for survivors"
// triggers: World.kill_cell, World.deposit_decay, World.record, Genome.decode
// because: "Toxin only forms where deaths cluster (>= toxin_generation_threshold within a radius), so mass die-offs poison their own ground — a negative-feedback brake on overcrowding"
fn phase_cleanup(world: &mut World, config: &WorldConfig, cache: &mut DecodeCache) {
    let width = world.width;
    let height = world.height;

    // Collect dead and living cells from the next grid, in tile order (the
    // order dead slots are freed decides which ids later births reuse).
    let mut dead_positions: Vec<(u16, u16, u32)> = Vec::new();
    let mut living_cells: Vec<(u32, usize)> = Vec::new();

    for (cell_id, i) in world.placed_in_next() {
        let cell = world.get_cell(cell_id);
        if !cell.is_alive() {
            let x = (i % width as usize) as u16;
            let y = (i / width as usize) as u16;
            dead_positions.push((x, y, cell_id));
        } else {
            living_cells.push((cell_id, i));
        }
    }

    // Dead cell processing
    for &(x, y, cell_id) in &dead_positions {
        let cell = world.get_cell(cell_id);
        // Structural matter plus whatever energy is left. `abs()` used to turn
        // combat overkill into a bonus and left starved cells with ~nothing,
        // so scavengers had no food supply at all.
        let decay_deposit =
            config.corpse_biomass + cell.energy.max(0.0) * config.corpse_energy_fraction;
        let cause = world.record(cell_id).death.unwrap_or(DeathCause::Other);
        world.stats.record_death(cause);
        world.stats.decay_deposited += decay_deposit as f64;
        let cell = world.get_cell(cell_id);
        // How long this body stays edible is its own decay_rate gene
        // (spec.md 30). Where a tile already holds matter from an earlier
        // corpse, blend the two rates by how much energy each brought.
        let deposit_fade =
            energy::corpse_decay_fade(cache.get(cell_id, &cell.genome, config), config);
        let idx = world.tile_index(x, y);
        world.deposit_decay(idx, decay_deposit, deposit_fade);
        world.next_grid_mut()[idx].cell_id = 0;
        world.kill_cell(cell_id);
    }

    // Toxin generation at death clusters
    let radius = config.toxin_generation_radius;
    for &(dx, dy, _) in &dead_positions {
        let mut death_count: u32 = 0;
        for &(ox, oy, _) in &dead_positions {
            let dist_x = (dx as i32 - ox as i32)
                .abs()
                .min(width as i32 - (dx as i32 - ox as i32).abs());
            let dist_y = (dy as i32 - oy as i32)
                .abs()
                .min(height as i32 - (dy as i32 - oy as i32).abs());
            if dist_x <= radius as i32 && dist_y <= radius as i32 {
                death_count += 1;
            }
        }
        if death_count >= config.toxin_generation_threshold {
            let idx = world.tile_index(dx, dy);
            world.next_grid_mut()[idx].toxin += death_count as f32 * 0.5;
        }
    }

    // Living cell upkeep
    for &(cell_id, tile_idx) in &living_cells {
        let cell_mut = world.get_cell_mut(cell_id);
        cell_mut.age = cell_mut.age.saturating_add(1);
        cell_mut.cooldown_remaining = cell_mut.cooldown_remaining.saturating_sub(1);

        let decoded = cache
            .get(cell_id, &world.get_cell(cell_id).genome.clone(), config)
            .clone();
        let mut emission = decoded.get(genome::SIGNAL_EMISSION);
        // spec.md gene 42, swarm_signal: "emits a rally pheromone when food
        // is found". The gene was never read. Food here is what the cell's
        // own strategy eats, the same test `sense` uses, so a photosynthesizer
        // rallies on light and a scavenger on a corpse.
        let tile = world.next_grid()[tile_idx];
        let found_food = if decoded.get(genome::SCAVENGE_ABILITY)
            >= decoded.get(genome::PHOTOSYNTHESIS_RATE)
            && decoded.get(genome::SCAVENGE_ABILITY) >= decoded.get(genome::THERMOSYNTHESIS_RATE)
        {
            tile.decay_energy > 0.0
        } else if decoded.get(genome::PHOTOSYNTHESIS_RATE)
            >= decoded.get(genome::THERMOSYNTHESIS_RATE)
        {
            tile.sunlight > 128
        } else {
            let (x, y) = world.get_cell(cell_id).position;
            world.is_in_vent_zone(x, y, config.vent_radius)
        };
        if found_food {
            emission += decoded.get(genome::SWARM_SIGNAL);
        }
        if emission > 0.0 {
            world.next_grid_mut()[tile_idx].pheromone += emission;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;
    use crate::sim::cell::Cell;
    use crate::sim::genome::Genome;
    use crate::sim::world::World;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn small_config() -> WorldConfig {
        WorldConfig {
            grid_width: 16,
            grid_height: 16,
            // No vents: at the default vent_radius a 16x16 world is entirely
            // inside the vent zone, which would feed cells these tests need
            // to starve.
            vent_count: 0,
            initial_cell_count: 0,
            ..WorldConfig::default()
        }
    }

    #[test]
    fn run_tick_advances_counter() {
        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        assert_eq!(world.tick, 0);
        run_tick(&mut world, &config, &mut rng);
        assert_eq!(world.tick, 1);
    }

    #[test]
    fn a_scavenger_standing_on_decay_actually_eats_it() {
        // "Scavengers don't appear to be eating at all" — check the whole
        // path end to end: the energy phase reads the cell's own tile in the
        // *next* grid, credits the income, and subtracts what it consumed
        // from that same tile.
        let config = WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 0,
            initial_decay_matter: 0.0,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);

        let mut data = [6u8; genome::GENOME_LEN];
        data[genome::SCAVENGE_ABILITY] = 255;
        data[genome::ENERGY_STORAGE_CAP] = 200;
        data[genome::SPEED] = 0;
        data[genome::DORMANCY_TRIGGER] = 0;
        // Below `reproduction_energy_floor`, so the tick measures eating and
        // not a split: this cell used to reproduce here too, which only went
        // unnoticed while a child could be born with ~0.1 energy.
        let start = config.reproduction_energy_floor - 10.0;
        let id = world.spawn_cell(Cell::new(Genome::new(data), start, (5, 5)));
        world.set_current_tile_cell_id(5, 5, id);
        let idx = world.tile_index(5, 5);
        world.current_grid_mut()[idx].decay_energy = 20.0;
        world.current_grid_mut()[idx].decay_fade = config.decay_rate;

        let before = world.get_cell(id).energy;
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        run_tick(&mut world, &config, &mut rng);

        let after = world.get_cell(id).energy;
        let left = world.current_grid()[world.tile_index(5, 5)].decay_energy;
        assert!(
            after > before,
            "a scavenger on 20 decay went from {before} to {after} — it is not eating"
        );
        assert!(
            left < 20.0,
            "the tile still holds {left} decay, so nothing was consumed"
        );
    }

    #[test]
    fn a_tough_corpse_outlasts_a_fragile_one() {
        // spec.md gene 30, decay_rate: "how long the cell's corpse persists
        // as scavengeable decay matter". The gene was never read, so every
        // corpse faded at the one world rate and a scavenger had no reason
        // to prefer any body over another.
        let config = WorldConfig {
            grid_width: 8,
            grid_height: 8,
            vent_count: 0,
            initial_cell_count: 0,
            initial_decay_matter: 0.0,
            ..WorldConfig::default()
        };
        let corpse_at = |gene: u8, x: u16| {
            let mut world = World::new(&config);
            let mut data = [0u8; genome::GENOME_LEN];
            data[genome::DECAY_RATE] = gene;
            let id = world.spawn_cell(Cell::new(Genome::new(data), 0.0, (x, 0)));
            world.set_current_tile_cell_id(x, 0, id);
            world.prepare_next();
            let idx = world.tile_index(x, 0);
            world.next_grid_mut()[idx].cell_id = id;
            phase_cleanup(&mut world, &config, &mut DecodeCache::default());
            world.swap_buffers();
            for _ in 0..200 {
                crate::sim::diffusion::fade_decay(world.current_grid_mut(), config.decay_rate);
            }
            world.current_grid()[idx].decay_energy
        };

        let tough = corpse_at(0, 1);
        let fragile = corpse_at(255, 2);
        assert!(
            tough > fragile * 2.0,
            "a decay_rate-0 corpse left {tough:.3} after 200 ticks against {fragile:.3} for a \
             decay_rate-255 one; the gene is not reaching the tile"
        );
    }

    #[test]
    fn starved_corpse_still_leaves_biomass() {
        // A starved cell has no energy left but still has a body. Without
        // the structural floor, starvation deaths left ~0 decay and the
        // scavenger niche had no food supply.
        use crate::sim::cell::Cell;
        use crate::sim::genome::{GENOME_LEN, Genome};

        // No seeded detritus: this test is about what a corpse leaves, and a
        // cell that can scavenge the starting decay does not starve to make one.
        let config = WorldConfig {
            initial_decay_matter: 0.0,
            ..small_config()
        };
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // The cell has to actually starve: no acquisition genes at all, and
        // dormancy off, since a hibernating cell does not die to make a corpse.
        let mut data = [50u8; GENOME_LEN];
        data[crate::sim::genome::DORMANCY_TRIGGER] = 0;
        for g in [
            genome::PHOTOSYNTHESIS_RATE,
            genome::THERMOSYNTHESIS_RATE,
            genome::SCAVENGE_ABILITY,
            genome::PREDATION_EFFICIENCY,
        ] {
            data[g] = 0;
        }
        let cell = Cell::new(Genome::new(data), 0.05, (5, 5));
        let cell_id = world.spawn_cell(cell);
        world.set_current_tile_cell_id(5, 5, cell_id);

        run_tick(&mut world, &config, &mut rng);

        let idx = world.tile_index(5, 5);
        let decay = world.current_grid()[idx].decay_energy;
        assert!(
            decay >= config.corpse_biomass,
            "starved corpse left {decay}, expected at least {}",
            config.corpse_biomass
        );
    }

    /// `docs/spec.md`, Decay Matter: a corpse leaves `corpse_biomass` plus
    /// `corpse_energy_fraction` of whatever energy remained. Senescence
    /// zeroed the energy before cleanup, so a well-fed cell dying of old age
    /// left only the biomass. With `corpses_keep_energy` it leaves both.
    #[test]
    fn an_old_corpse_leaves_its_remaining_energy_too() {
        use crate::sim::cell::Cell;
        use crate::sim::genome::{GENOME_LEN, Genome};

        let decay_left = |keep: bool| {
            let config = WorldConfig {
                corpses_keep_energy: keep,
                initial_decay_matter: 0.0,
                ..small_config()
            };
            let mut world = World::new(&config);
            // max_age 0: its lifespan is `min_lifespan_ticks`, reached now.
            // An all-zero genome earns nothing and costs almost nothing, and
            // the cooldown keeps it from splitting before it dies.
            let mut cell = Cell::new(Genome::new([0u8; GENOME_LEN]), 50.0, (5, 5));
            cell.age = config.min_lifespan_ticks;
            cell.cooldown_remaining = 10;
            let id = world.spawn_cell(cell);
            world.set_current_tile_cell_id(5, 5, id);
            run_tick(&mut world, &config, &mut ChaCha8Rng::seed_from_u64(1));
            assert_eq!(world.population(), 0, "the cell should die of old age");
            (
                world.current_grid()[world.tile_index(5, 5)].decay_energy,
                config,
            )
        };

        let (lost, config) = decay_left(false);
        assert!((lost - config.corpse_biomass).abs() < 1e-3);
        let (kept, _) = decay_left(true);
        // It died holding just under 50; half of that is its energy share.
        assert!(
            kept - lost > config.corpse_energy_fraction * 45.0,
            "an old corpse left {kept}, only {} more than the bare biomass",
            kept - lost
        );
    }

    #[test]
    fn dead_cell_produces_decay_and_frees_slot() {
        use crate::sim::cell::Cell;
        use crate::sim::genome::{GENOME_LEN, Genome};

        let config = WorldConfig {
            initial_decay_matter: 0.0,
            ..small_config()
        };
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Cell with barely any energy and no way to earn more — it will die
        // from metabolism. dormancy off, or it hibernates instead of dying.
        let mut data = [50u8; GENOME_LEN];
        data[crate::sim::genome::DORMANCY_TRIGGER] = 0;
        for g in [
            genome::PHOTOSYNTHESIS_RATE,
            genome::THERMOSYNTHESIS_RATE,
            genome::SCAVENGE_ABILITY,
            genome::PREDATION_EFFICIENCY,
        ] {
            data[g] = 0;
        }
        let cell = Cell::new(Genome::new(data), 0.1, (5, 5));
        let cell_id = world.spawn_cell(cell);
        world.set_current_tile_cell_id(5, 5, cell_id);

        let pop_before = world.population();
        run_tick(&mut world, &config, &mut rng);

        assert!(
            world.population() < pop_before,
            "population should decrease after cell death"
        );

        let total = (config.grid_width * config.grid_height) as usize;
        let total_decay: f32 = (0..total)
            .map(|i| world.current_grid()[i].decay_energy)
            .sum();
        assert!(total_decay > 0.0, "dead cell should leave decay matter");
    }
}
