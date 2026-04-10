// Tick orchestration: calls each phase in order, manages buffer swaps

use rand_chacha::ChaCha8Rng;

use crate::config::WorldConfig;
use crate::sim::actions::{self, Action};
use crate::sim::diffusion;
use crate::sim::energy::{self, EnergyContext};
use crate::sim::genome::{self};
use crate::sim::phase::{self, PhaseInput};
use crate::sim::world::World;

/// Execute one full simulation tick.
///
/// Phases run in strict order:
/// 1. Diffusion & environment (diffuse fields, recompute sunlight)
/// 2. Sensing + phase evaluation + decision
/// 3. Action resolution
/// 4. Energy update
/// 5. Cleanup (dead → decay, toxin generation, pheromone emission)
/// 6. Bookkeeping (swap buffers, clear next, increment tick)
pub fn run_tick(world: &mut World, config: &WorldConfig, rng: &mut ChaCha8Rng) {
    // Phase 1: Diffusion & environment
    diffusion::run_diffusion_phase(world, config);
    world.update_sunlight(config.sunlight_gradient_strength);

    // Phase 2: Sensing + phase evaluation + decision
    let actions = phase_sense_decide(world, config, rng);

    // Phase 3: Action resolution
    actions::resolve_all(&actions, world, config, rng);

    // Phase 4: Energy update
    phase_energy_update(world, config);

    // Phase 5: Cleanup
    phase_cleanup(world, config);

    // Phase 6: Bookkeeping
    world.swap_buffers();
    world.clear_next();
    world.tick += 1;
}

/// Phase 2: For each living cell, sense → evaluate phase → decide action.
fn phase_sense_decide(
    world: &mut World,
    config: &WorldConfig,
    rng: &mut ChaCha8Rng,
) -> Vec<(u32, Action)> {
    let cell_ids = world.cell_ids();
    let mut action_buffer: Vec<(u32, Action)> = Vec::with_capacity(cell_ids.len());

    for cell_id in cell_ids {
        let cell = world.get_cell(cell_id);
        let mut decoded = cell.genome.decode(config);

        // Sense
        let sense = actions::sense(cell, &decoded, world);

        // Build PhaseInput
        let energy_cap = decoded.get(genome::ENERGY_STORAGE_CAP) * 255.0;
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
            maturity_threshold: (decoded.get(genome::MATURITY_AGE) * 1000.0) as u32,
            memory_length: (decoded.get(genome::MEMORY_LENGTH) * 255.0) as u32,
        };

        // Evaluate phase transition
        let new_phase = phase::evaluate_phase(&cell.genome, &phase_input, cell.active_phase);

        // Update cell phase state
        let cell_mut = world.get_cell_mut(cell_id);
        if new_phase != cell_mut.active_phase {
            cell_mut.active_phase = new_phase;
            cell_mut.phase_ticks = 0;
        } else {
            cell_mut.phase_ticks = cell_mut.phase_ticks.saturating_add(1);
        }

        // Apply phase modifiers — re-borrow cell immutably
        let cell = world.get_cell(cell_id);
        phase::apply_phase_modifiers(&mut decoded, &cell.genome, new_phase);

        // Decide action
        let action = actions::decide(cell, &decoded, &sense, rng);
        action_buffer.push((cell_id, action));
    }

    action_buffer
}

/// Phase 4: Energy update for all living cells in the next grid.
fn phase_energy_update(world: &mut World, config: &WorldConfig) {
    let width = world.width;
    let height = world.height;
    let tick = world.tick;

    // Pre-compute vent income: for each vent, find adjacent cells in
    // the next grid, compute per-cell share of thermo income.
    let mut vent_income_map: Vec<(u32, f32)> = Vec::new();
    for &vx in &world.vent_positions.clone() {
        // Vent sits at bottom row (y = height - 1)
        let vy = height as i32 - 1;
        let mut adjacent_cells: Vec<u32> = Vec::new();

        // Check radius 1 around vent position
        for dy in -1..=1_i32 {
            for dx in -1..=1_i32 {
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
            let decoded = cell.genome.decode(config);
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

    // Scan next grid for living cells and update energy
    let total = (width * height) as usize;
    let mut live_cells: Vec<(u32, usize)> = Vec::new();
    for i in 0..total {
        let cell_id = world.next_grid()[i].cell_id;
        if cell_id != 0 && world.get_cell(cell_id).is_alive() {
            live_cells.push((cell_id, i));
        }
    }

    for (cell_id, tile_idx) in live_cells {
        let tile = world.next_grid()[tile_idx];
        let cell = world.get_cell(cell_id);
        let mut decoded = cell.genome.decode(config);
        phase::apply_phase_modifiers(&mut decoded, &cell.genome, cell.active_phase);

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

        if result.decay_consumed > 0.0 {
            world.next_grid_mut()[tile_idx].decay_energy -= result.decay_consumed;
            if world.next_grid_mut()[tile_idx].decay_energy < 0.0 {
                world.next_grid_mut()[tile_idx].decay_energy = 0.0;
            }
        }
    }
}

/// Phase 5: Cleanup — process dead cells, generate toxin, update living cells.
fn phase_cleanup(world: &mut World, config: &WorldConfig) {
    let width = world.width;
    let height = world.height;
    let total = (width * height) as usize;

    // Collect dead and living cells from the next grid
    let mut dead_positions: Vec<(u16, u16, u32)> = Vec::new();
    let mut living_cells: Vec<(u32, usize)> = Vec::new();

    for i in 0..total {
        let cell_id = world.next_grid()[i].cell_id;
        if cell_id == 0 {
            continue;
        }
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
        let decay_deposit = cell.energy.abs() * 0.5;
        let idx = world.tile_index(x, y);
        world.next_grid_mut()[idx].decay_energy += decay_deposit;
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

        let decoded = cell_mut.genome.decode(config);
        let emission = decoded.get(genome::SIGNAL_EMISSION);
        if emission > 0.0 {
            world.next_grid_mut()[tile_idx].pheromone += emission;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;
    use crate::sim::world::World;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn small_config() -> WorldConfig {
        WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 1,
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
    fn dead_cell_produces_decay_and_frees_slot() {
        use crate::sim::cell::Cell;
        use crate::sim::genome::{GENOME_LEN, Genome};

        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Cell with barely any energy — will die from metabolism
        let genome = Genome::new([50u8; GENOME_LEN]);
        let cell = Cell::new(genome, 0.1, (5, 5));
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
