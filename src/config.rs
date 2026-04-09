use serde::{Deserialize, Serialize};

/// Strategy for initial cell placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedStrategy {
    RandomUniform,
    RandomClusters,
    PresetArchetypes,
}

/// All world parameters. Loaded from JSON at startup, immutable during a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldConfig {
    // Grid
    pub grid_width: u32,
    pub grid_height: u32,

    // Energy sources
    pub sunlight_gradient_strength: f32,
    pub vent_count: u32,
    pub vent_output: f32,
    /// (active_ticks, dormant_ticks). (0, 0) = always on.
    pub vent_cycle: (u32, u32),

    // Diffusion
    pub pheromone_decay: f32,
    pub pheromone_diffusion: f32,
    pub toxin_decay: f32,
    pub toxin_diffusion: f32,
    pub toxin_generation_threshold: u32,
    /// Radius for counting nearby deaths when generating toxin at death sites.
    pub toxin_generation_radius: u32,

    // Decay
    pub decay_rate: f32,

    // Temperature
    pub temperature_noise_scale: f32,
    pub temperature_mismatch_cost: f32,
    pub temperature_diffusion: f32,
    pub temperature_decay: f32,

    // Expression constraints
    pub top_n_gene_count: u32,
    pub top_n_falloff: f32,
    pub metabolic_cost_exponent: f32,

    // Seeding
    pub initial_cell_count: u32,
    pub initial_genome_strategy: SeedStrategy,
    pub min_viable_acquisition: u8,
    pub base_spawn_energy: f32,
    pub bonus_spawn_energy: f32,
    pub cluster_count: u32,
    pub seed: u64,

    // Simulation
    pub max_ticks: Option<u64>,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            grid_width: 512,
            grid_height: 512,

            sunlight_gradient_strength: 1.0,
            vent_count: 5,
            vent_output: 8.0,
            vent_cycle: (0, 0),

            pheromone_decay: 0.05,
            pheromone_diffusion: 0.15,
            toxin_decay: 0.005,
            toxin_diffusion: 0.02,
            toxin_generation_threshold: 10,
            toxin_generation_radius: 2,

            decay_rate: 0.02,

            temperature_noise_scale: 0.01,
            temperature_mismatch_cost: 0.3,
            temperature_diffusion: 0.001,
            temperature_decay: 0.0001,

            top_n_gene_count: 12,
            top_n_falloff: 0.1,
            metabolic_cost_exponent: 1.5,

            initial_cell_count: 5000,
            initial_genome_strategy: SeedStrategy::RandomClusters,
            min_viable_acquisition: 40,
            base_spawn_energy: 50.0,
            bonus_spawn_energy: 150.0,
            cluster_count: 16,
            seed: 42,

            max_ticks: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes_roundtrip() {
        let config = WorldConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: WorldConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.grid_width, config.grid_width);
        assert_eq!(parsed.seed, config.seed);
    }
}
