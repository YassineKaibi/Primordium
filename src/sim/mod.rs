pub mod actions;
pub mod cell;
pub mod diffusion;
pub mod energy;
pub mod genome;
pub mod phase;
pub mod spawner;
pub mod stats;
pub mod tick;
pub mod world;

// @veridikt
// kind: module
// name: Sim
// purpose: "Public simulation facade: owns the world, config, and seeded RNG, and exposes new()/step()/snapshot() to the binary and tests"
// owner: "primordium-maintainers"
// because: "The RNG lives here and is threaded into every stochastic call, so a given seed+config fully determines the run — the determinism guarantee the project is built around"
// depends_on: World, Spawner, Tick

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use self::world::{World, WorldSnapshot};
use crate::config::WorldConfig;

/// Top-level simulation controller. Owns the world, config, and RNG.
pub struct Simulation {
    world: World,
    config: WorldConfig,
    rng: ChaCha8Rng,
    /// Reused across ticks so its allocation is not rebuilt every tick.
    /// Cleared at the top of each tick, so it holds no cross-tick state.
    decode_cache: genome::DecodeCache,
}

impl Simulation {
    /// Create a new simulation from config. Seeds the world with initial cells.

    // @veridikt
    // purpose: "Construct a simulation: build the world, seed the RNG from config.seed, and place founder cells"
    // triggers: World.new, Spawner.seed_world
    // because: "RNG is seeded from config.seed before seeding so the founder population is reproducible"
    pub fn new(config: WorldConfig) -> Self {
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
        spawner::seed_world(&mut world, &config, &mut rng);
        Self {
            world,
            config,
            rng,
            decode_cache: genome::DecodeCache::default(),
        }
    }

    /// Advance the simulation by one tick.

    // @veridikt
    // purpose: "Advance the simulation one tick by delegating to the tick orchestrator with the owned world/config/rng"
    // triggers: Tick.run_tick_cached
    pub fn step(&mut self) {
        tick::run_tick_cached(
            &mut self.world,
            &self.config,
            &mut self.rng,
            &mut self.decode_cache,
        );
    }

    /// Produce a lightweight snapshot of the current world state.

    // @veridikt
    // purpose: "Expose a render-ready snapshot of the current world to the caller (the render thread)"
    // triggers: World.snapshot
    pub fn snapshot(&self) -> WorldSnapshot {
        self.world.snapshot(&self.config)
    }

    /// Read-only access to the world, for observers (the lab, tests).
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Introduce `count` copies of `genome` (all but the first mutated) into
    /// the band `[top, top + depth)` as one new lineage. Returns `(lineage,
    /// cells placed)`. See `spawner::inject`.

    // @veridikt
    // purpose: "Let the lab drop a batch of new cells into a running world, using the simulation's own RNG"
    // triggers: Spawner.inject
    // because: "The invasion-from-rare assay needs to add a strategy mid-run; routing it through the owned ChaCha8Rng keeps the run reproducible from its seed"
    pub fn inject(
        &mut self,
        genome: &genome::Genome,
        count: usize,
        top: i32,
        depth: usize,
    ) -> (u32, usize) {
        spawner::inject(
            &mut self.world,
            &self.config,
            &mut self.rng,
            genome,
            count,
            top,
            depth,
        )
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

    /// Every cell that appears or disappears is accounted for by the tick's
    /// own counters: population(t+1) = population(t) + births - deaths, and
    /// every death has a known cause. A path to zero energy the stats did not
    /// see would break this (the first draft missed stillbirths and deaths in
    /// childbirth, and counted them as "other"), and so would a death counted
    /// when the blow lands for a cell that is then revived.
    #[test]
    fn every_birth_and_death_is_counted_and_every_death_has_a_cause() {
        use crate::sim::stats::DeathCause;
        for strategy in [
            crate::config::SeedStrategy::RandomUniform,
            crate::config::SeedStrategy::RandomClusters,
            crate::config::SeedStrategy::PresetArchetypes,
        ] {
            let config = WorldConfig {
                grid_width: 64,
                grid_height: 64,
                vent_count: 3,
                initial_cell_count: 400,
                initial_genome_strategy: strategy.clone(),
                ..WorldConfig::default()
            };
            let mut sim = Simulation::new(config);
            let (mut births, mut deaths) = (0, 0);
            for _ in 0..150 {
                let before = sim.world().population();
                sim.step();
                let s = &sim.world().stats;
                assert_eq!(
                    sim.world().population(),
                    before + s.births - s.total_deaths(),
                    "{strategy:?} tick {}: population does not add up",
                    sim.world().tick
                );
                assert_eq!(s.deaths[DeathCause::Other as usize], 0, "{strategy:?}");
                // A cell struck to zero and revived in the same tick (by kin
                // sharing, or by absorbing its own kill) is alive, and must
                // not carry the label of a death it did not die.
                for (id, _) in sim.world().live() {
                    assert_eq!(
                        sim.world().record(id).death,
                        None,
                        "{strategy:?}: living cell {id} carries a death cause"
                    );
                }
                births += s.births;
                deaths += s.total_deaths();
            }
            assert!(births > 0 && deaths > 0, "{strategy:?}: nothing happened");
        }
    }

    /// A child joins its parent's founder lineage, and lineage survives any
    /// number of generations; an injected batch is a lineage of its own.
    #[test]
    fn lineage_is_inherited_and_injection_starts_a_new_one() {
        let config = WorldConfig {
            grid_width: 64,
            grid_height: 64,
            vent_count: 3,
            initial_cell_count: 200,
            ..WorldConfig::default()
        };
        let mut sim = Simulation::new(config);
        let founders: std::collections::HashSet<u32> = sim
            .world()
            .live()
            .iter()
            .map(|(id, _)| sim.world().record(*id).lineage)
            .collect();
        assert_eq!(
            founders.len(),
            200,
            "every uniform founder is its own lineage"
        );
        assert!(!founders.contains(&0), "no founder is left untagged");

        let mut born = 0;
        for _ in 0..200 {
            sim.step();
            born += sim.world().stats.births;
        }
        assert!(born > 0);
        for (id, _) in sim.world().live() {
            let l = sim.world().record(id).lineage;
            assert!(
                founders.contains(&l),
                "cell {id} has lineage {l}, not a founder's"
            );
        }

        let genome = spawner::archetype_genome(spawner::Archetype::Predator);
        let (lineage, placed) = sim.inject(&genome, 10, 20, 4);
        assert_eq!(placed, 10);
        assert!(!founders.contains(&lineage));
        let injected: Vec<(u16, u16)> = sim
            .world()
            .live()
            .iter()
            .filter(|(id, _)| sim.world().record(*id).lineage == lineage)
            .map(|(_, c)| c.position)
            .collect();
        assert_eq!(injected.len(), 10);
        assert!(injected.iter().all(|&(_, y)| (20..24).contains(&y)));
        assert_eq!(sim.world().integrity(), (0, 0));
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
