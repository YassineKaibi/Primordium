// Allow dead code during development: modules are tested but not yet
// wired into the binary's main loop.
#![allow(dead_code)]

pub mod actions;
pub mod cell;
pub mod diffusion;
pub mod energy;
pub mod genome;
pub mod phase;
pub mod spawner;
pub mod tick;
pub mod world;

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use self::world::{World, WorldSnapshot};
use crate::config::WorldConfig;

/// Top-level simulation controller. Owns the world, config, and RNG.
pub struct Simulation {
    world: World,
    config: WorldConfig,
    rng: ChaCha8Rng,
}

impl Simulation {
    /// Create a new simulation from config. Seeds the world with initial cells.
    pub fn new(config: WorldConfig) -> Self {
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
        spawner::seed_world(&mut world, &config, &mut rng);
        Self { world, config, rng }
    }

    /// Advance the simulation by one tick.
    pub fn step(&mut self) {
        tick::run_tick(&mut self.world, &self.config, &mut self.rng);
    }

    /// Produce a lightweight snapshot of the current world state.
    pub fn snapshot(&self) -> WorldSnapshot {
        self.world.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config() -> WorldConfig {
        WorldConfig {
            grid_width: 32,
            grid_height: 32,
            vent_count: 1,
            initial_cell_count: 20,
            cluster_count: 2,
            ..WorldConfig::default()
        }
    }

    #[test]
    fn simulation_new_seeds_world() {
        let config = small_config();
        let sim = Simulation::new(config);
        let snap = sim.snapshot();
        assert!(
            snap.stats.population > 0,
            "world should have cells after seeding"
        );
    }

    #[test]
    fn simulation_step_advances_tick() {
        let config = small_config();
        let mut sim = Simulation::new(config);
        assert_eq!(sim.snapshot().tick, 0);
        sim.step();
        assert_eq!(sim.snapshot().tick, 1);
    }

    #[test]
    fn determinism_same_seed_same_result() {
        let config = small_config();
        let mut sim_a = Simulation::new(config.clone());
        let mut sim_b = Simulation::new(config);

        for _ in 0..10 {
            sim_a.step();
            sim_b.step();
        }

        let snap_a = sim_a.snapshot();
        let snap_b = sim_b.snapshot();

        assert_eq!(snap_a.tick, snap_b.tick);
        assert_eq!(snap_a.stats.population, snap_b.stats.population);
        assert_eq!(snap_a.cells.len(), snap_b.cells.len());
        assert!(
            (snap_a.stats.total_energy - snap_b.stats.total_energy).abs() < f64::EPSILON,
            "energy must match: {} vs {}",
            snap_a.stats.total_energy,
            snap_b.stats.total_energy,
        );
        for (a, b) in snap_a.cells.iter().zip(snap_b.cells.iter()) {
            assert_eq!(a, b, "cell data must be identical");
        }
        for (a, b) in snap_a.decay_map.iter().zip(snap_b.decay_map.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "decay maps must be bit-identical");
        }
    }
}
