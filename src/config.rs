// @veridikt
// kind: module
// name: Config
// purpose: "The full tuning surface for a run: grid size, energy-source rates, diffusion/decay, expression constraints, seeding, and the RNG seed — loaded from JSON, immutable during a run"
// owner: "primordium-maintainers"
// because: "Every balance knob lives here so a run is fully described by one serializable struct; combined with the seed this is what makes simulations reproducible and shareable"

use serde::{Deserialize, Serialize};

/// Strategy for initial cell placement.

// @veridikt
// kind: type
// purpose: "Selects how founder cells are placed: uniform scatter, related random clusters, or the four hand-designed archetype bands"
// because: "RandomClusters is the default because it seeds genetic relatedness that kin behaviors need; PresetArchetypes trades that randomness for a controlled start with the trophic links already in contact"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedStrategy {
    RandomUniform,
    RandomClusters,
    PresetArchetypes,
}

/// All world parameters. Loaded from JSON at startup, immutable during a run.

// @veridikt
// kind: type
// purpose: "The complete immutable parameter set for one run; the expression-constraint knobs (top_n_gene_count, top_n_falloff, metabolic_cost_exponent) are what keep evolution from collapsing into supercells"
// because: "vent_cycle (0,0) is treated as always-on so the simplest config needs no special casing; max_ticks is optional so runs can be open-ended or bounded; serde(default) fills any field a file leaves out from Default, so a config written before a field existed still loads; deny_unknown_fields keeps that from turning a misspelled key into a silent default"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    // Grid
    pub grid_width: u32,
    pub grid_height: u32,

    // Energy sources
    pub sunlight_gradient_strength: f32,
    /// Beer-Lambert absorption one occupied tile adds to its light column,
    /// on top of the water's. At 0.2 a cell takes 18% of the light from
    /// everything below it, so a colony a few rows deep shades itself out
    /// and life sits in a band of ~9-35 rows.
    pub cell_light_absorption: f32,
    pub vent_count: u32,
    pub vent_output: f32,
    /// How far a vent's output reaches, in tiles (Chebyshev). The bottom
    /// zone is a zone, not a 3x3 patch: with a radius of 1 no founder
    /// cluster ever came within 47 tiles of a vent.
    pub vent_radius: u32,
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
    /// Share of each tile's decay matter that sinks one row toward the vents
    /// every tick. At 0 decay stays where the corpse fell, so it feeds only
    /// that tile.
    pub decay_sink_rate: f32,
    /// Decay matter every tile starts the run with. Without it the scavenger
    /// niche does not exist until the first cells die of old age, roughly
    /// 300-1300 ticks in, and a scavenger's runway is its storage cap over
    /// its upkeep — about 157 ticks. Scavengers seeded at tick 0 therefore
    /// starved before their food supply existed.
    pub initial_decay_matter: f32,
    /// Multiplier on `decay_rate` for a corpse whose `decay_rate` gene is 0 —
    /// the toughest body, which persists longest as scavengeable matter.
    pub corpse_decay_scale_min: f32,
    /// Multiplier on `decay_rate` for a corpse whose `decay_rate` gene is 1.
    pub corpse_decay_scale_max: f32,

    // Temperature
    pub temperature_noise_scale: f32,
    pub temperature_mismatch_cost: f32,
    pub temperature_diffusion: f32,
    pub temperature_decay: f32,

    // Expression constraints
    pub top_n_gene_count: u32,
    pub top_n_falloff: f32,
    pub metabolic_cost_exponent: f32,
    /// Multiplier on the summed per-gene expression cost. The spec's shape
    /// (every gene costs, superlinearly) at a level a specialist can pay.
    pub metabolic_cost_scale: f32,
    /// When true, the genes the simulation only reads raw from the genome
    /// bytes (`genome::RAW_READ_GENES`: mutation rate and magnitude, gene
    /// linkage, transposon rate, horizontal transfer, aggression trigger,
    /// kin recognition precision) are left out of top-N gating and of
    /// metabolic cost. Their decoded values drive nothing. When false they
    /// still take top-N slots and cost upkeep: the archetype predator spends
    /// two of its 12 slots on the mutation genes, which gates its `max_age`
    /// to a 401-tick life (handover step 17).
    pub raw_genes_outside_expression: bool,

    // Heredity
    /// Per-byte mutation probability when `mutation_rate` is at its maximum.
    /// The gene scales within this bound, so meta-evolution still works but a
    /// founder cannot rewrite half its genome per birth.
    pub max_mutation_rate: f32,
    /// Largest byte shift a mutation can apply when `mutation_magnitude` is
    /// at its maximum.
    pub max_mutation_magnitude: u8,
    /// Chance per birth of one transposon event when `transposon_rate` is at
    /// its maximum. Bounded like `max_mutation_rate`, so meta-evolution
    /// works without a founder shredding its own genome.
    pub max_transposon_rate: f32,
    /// Chance per kill of absorbing one of the victim's genes when
    /// `horizontal_transfer` is at its maximum.
    pub max_horizontal_transfer: f32,

    // Movement and behaviour
    /// Furthest a moving cell travels in one tick, in tiles. `docs/spec.md`
    /// gene 6 makes `speed` the *probability* of moving; this sets how far a
    /// move goes: `max(1, round(speed * max_move_distance))` tiles along the
    /// chosen heading, stopping at the first occupied tile and never beyond
    /// the cell's own sense radius. 1 is the spec's one-tile-per-tick world.
    pub max_move_distance: u32,
    /// When true, a cell only takes the Attack action against a threat it can
    /// actually damage (its attack beats the target's armour); otherwise the
    /// threat falls through to the Flee gate. When false, any threat in range
    /// is attacked, however harmless the attacker — so prey "fights" an
    /// adjacent predator instead of running.
    pub attack_only_when_harmful: bool,
    /// Fullness above which a cell stops hunting: with energy above this
    /// fraction of its storage cap it takes no Attack action and falls
    /// through to Flee/Move. 1.0 turns the check off. Without it a
    /// self-replacing predator kept killing at the same rate however full it
    /// was and ate every archetype web out (handover steps 18-19).
    pub satiation_fraction: f32,
    /// When true, fleeing is a real escape. The Flee gate fires with
    /// probability `speed * flee_response` (a flight is a move, and takes the
    /// speed to make it), only from the nearest non-kin whose blow or venom
    /// would actually cost the cell energy, and every Flee and Move resolves
    /// before any blow lands, so a strike misses a target that has moved out
    /// of the attacker's reach. When false, any non-kin sends a cell with any
    /// `flee_response` running, with no roll, and the attack pass pins the
    /// defender in place before anyone moves, so an attacked cell never
    /// escapes.
    pub flee_can_escape: bool,
    /// When true, a cell steers toward the richest food in sense range (most
    /// decay, most light), nearest first on ties; when false, toward the
    /// nearest tile holding any food at all.
    pub food_targets_richest: bool,
    /// When true, a cell standing on food is less likely to walk off it: its
    /// chance of moving is scaled by `1 - chemotaxis_strength * share`, where
    /// `share` is its own tile's food against the richest other food tile in
    /// sense range. When false, a mobile forager moves on its speed roll
    /// whatever it stands on, and a scavenger walks off a corpse it could
    /// have eaten for eight ticks.
    pub foragers_stay_on_food: bool,

    // Adaptation
    /// Fraction of the gap between a cell's effective temperature preference
    /// and its tile's temperature that it closes per tick when its
    /// `adaptation_rate` gene is at maximum.
    pub max_adaptation_rate: f32,

    // Dormancy
    /// Highest energy fraction a `dormancy_trigger` of 255 can mean.
    /// Dormancy is a last resort, not an operating mode: letting the gene
    /// span the whole range put a random-genome population 56% asleep and
    /// dropped Move to 0.3% of all actions.
    pub max_dormancy_trigger: f32,
    /// Floor on the metabolic multiplier `dormancy_cost` can buy. Without
    /// it a cheap hibernator is effectively immortal, so it neither
    /// reproduces nor dies and simply occupies a tile forever.
    pub min_dormancy_cost: f32,

    // Lifecycle
    /// Lifespan of a cell whose `max_age` gene is 0.
    pub min_lifespan_ticks: u32,
    /// Lifespan of a cell whose `max_age` gene is 255.
    pub max_lifespan_ticks: u32,

    /// Ticks before reproduction unlocks for a cell whose `maturity_age`
    /// gene is 255, before the lifespan cap below applies.
    pub max_maturity_ticks: u32,
    /// Hard cap on maturity as a fraction of the cell's own lifespan. A
    /// cell that matures after it dies is sterile by construction, and a
    /// whole founder lineage can be wiped out that way.
    pub maturity_lifespan_fraction: f32,

    // Corpses
    /// Structural matter every corpse leaves behind, whatever its energy.
    /// A starved cell has no energy left but still has a body.
    pub corpse_biomass: f32,
    /// Share of a corpse's remaining energy that becomes decay matter.
    pub corpse_energy_fraction: f32,
    /// When true, the energy a dying cell still held stays with its body
    /// instead of vanishing: a killed cell's tile gets the part of its
    /// pre-blow energy its killer did not absorb, and a cell that dies of old
    /// age leaves `corpse_energy_fraction` of its energy like any other
    /// corpse (`docs/spec.md`, Decay Matter). When false, senescence zeroes
    /// the energy first, so an old corpse leaves only `corpse_biomass`, and
    /// whatever a kill does not pay the killer is lost.
    pub corpses_keep_energy: bool,

    // Predation
    /// Share of a victim's energy a kill pays when `predation_efficiency` is
    /// at its maximum. The gene scales within this bound, the way
    /// `mutation_rate` scales within `max_mutation_rate`. The gene has no
    /// antagonist, so at 1.0 a hunter can bank a victim's whole store per
    /// kill.
    pub max_predation_efficiency: f32,

    // Energy economy
    /// Income of a perfect photosynthesiser in full sunlight, per tick.
    pub photo_max_income: f32,
    /// Fraction of consumed decay matter that becomes cell energy.
    pub scavenge_efficiency: f32,
    /// Most decay matter one cell can strip from a tile in a single tick.
    /// `scavenge_ability` is a rate, not a swallow: without a cap a maxed
    /// scavenger clears the whole tile on the tick it arrives, so a corpse
    /// is one meal rather than a food source, and the niche only pays for
    /// a cell that can walk to a fresh body every tick.
    pub max_scavenge_per_tick: f32,
    /// Storage capacity of a cell whose `energy_storage_cap` gene is 0.
    /// Without a floor, most founders can hold less than one tick of income.
    pub energy_cap_floor: f32,
    /// Storage capacity of a cell whose `energy_storage_cap` gene is 1.
    pub energy_cap_max: f32,
    /// Absolute energy a cell needs before it may reproduce, whatever its
    /// storage cap. Stops tiny-cap cells from splitting at near-zero energy.
    pub reproduction_energy_floor: f32,
    /// Least share of its parent's energy a child is born with, whatever its
    /// `offspring_energy_share` gene says. A share of 0 made zero-energy
    /// children that counted as births, died at cleanup and left
    /// `corpse_biomass` of decay from nothing; at 0.05 a child of a parent at
    /// `reproduction_energy_floor` starts with 2 energy, which outlasts the
    /// tick it is born in.
    pub min_offspring_energy_share: f32,
    /// Most of its energy a parent can hand its child. At 1.0 the parent
    /// died in childbirth; 0.95 leaves it 2 energy at the reproduction floor.
    pub max_offspring_energy_share: f32,

    // Seeding
    pub initial_cell_count: u32,
    pub initial_genome_strategy: SeedStrategy,
    pub min_viable_acquisition: u8,
    pub base_spawn_energy: f32,
    pub bonus_spawn_energy: f32,
    pub cluster_count: u32,
    /// Depth in rows of one `preset_archetypes` band. `0` sizes each band
    /// automatically so its share of `initial_cell_count` fills about half
    /// its tiles. Bands are shallow because every occupied tile absorbs 0.2
    /// of the light column below it, so a deep producer colony shades itself
    /// out.
    pub archetype_band_depth: u32,
    /// How `initial_cell_count` is split between the four archetypes, in
    /// `spawner::ARCHETYPES` order (photosynthesizer, vent-feeder,
    /// scavenger, predator). Normalized, so only the ratios matter.
    /// `docs/spec.md` asks for equal populations, which is the default; a
    /// trophic pyramid is the other thing worth measuring, because equal
    /// numbers give the consumers a numerical response the producers cannot
    /// match.
    pub archetype_population_shares: [f32; 4],
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
            cell_light_absorption: 0.2,
            vent_count: 12,
            vent_output: 40.0,
            vent_radius: 6,
            vent_cycle: (0, 0),

            pheromone_decay: 0.05,
            pheromone_diffusion: 0.15,
            toxin_decay: 0.005,
            toxin_diffusion: 0.02,
            toxin_generation_threshold: 10,
            toxin_generation_radius: 2,

            decay_rate: 0.02,
            decay_sink_rate: 0.0,
            initial_decay_matter: 8.0,
            corpse_decay_scale_min: 0.1,
            corpse_decay_scale_max: 2.0,

            temperature_noise_scale: 0.01,
            temperature_mismatch_cost: 0.3,
            temperature_diffusion: 0.001,
            temperature_decay: 0.0001,

            top_n_gene_count: 12,
            top_n_falloff: 0.1,
            metabolic_cost_exponent: 1.5,

            metabolic_cost_scale: 0.2,
            raw_genes_outside_expression: false,

            max_mutation_rate: 0.05,
            max_mutation_magnitude: 24,
            max_transposon_rate: 0.05,
            max_horizontal_transfer: 0.05,

            max_move_distance: 1,
            attack_only_when_harmful: false,
            satiation_fraction: 1.0,
            flee_can_escape: false,
            food_targets_richest: false,
            foragers_stay_on_food: false,

            max_adaptation_rate: 0.02,

            max_dormancy_trigger: 0.35,
            min_dormancy_cost: 0.25,

            min_lifespan_ticks: 300,
            max_lifespan_ticks: 3000,

            max_maturity_ticks: 30,
            maturity_lifespan_fraction: 0.5,

            corpse_biomass: 25.0,
            corpse_energy_fraction: 0.5,
            corpses_keep_energy: false,

            max_predation_efficiency: 1.0,

            photo_max_income: 4.0,
            scavenge_efficiency: 0.9,
            max_scavenge_per_tick: 3.0,
            energy_cap_floor: 60.0,
            energy_cap_max: 255.0,
            reproduction_energy_floor: 40.0,
            min_offspring_energy_share: 0.05,
            max_offspring_energy_share: 0.95,

            initial_cell_count: 1000,
            initial_genome_strategy: SeedStrategy::RandomUniform,
            min_viable_acquisition: 40,
            base_spawn_energy: 50.0,
            bonus_spawn_energy: 150.0,
            cluster_count: 16,
            archetype_band_depth: 0,
            archetype_population_shares: [0.25; 4],
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

    /// Every field added to `WorldConfig` used to break every config file
    /// written before it: step 14's four fields made both benchmark configs
    /// fail to parse ("missing field `max_move_distance`"). A missing field
    /// now takes its default.
    #[test]
    fn a_config_file_missing_newer_fields_still_loads() {
        let partial: WorldConfig =
            serde_json::from_str(r#"{ "grid_width": 64, "seed": 7 }"#).unwrap();
        assert_eq!(partial.grid_width, 64);
        assert_eq!(partial.seed, 7);
        assert_eq!(partial.grid_height, WorldConfig::default().grid_height);

        // A file written before step 14: every field but the four it added.
        let mut legacy = serde_json::to_value(WorldConfig::default()).unwrap();
        let obj = legacy.as_object_mut().unwrap();
        for field in [
            "max_move_distance",
            "attack_only_when_harmful",
            "food_targets_richest",
            "max_adaptation_rate",
        ] {
            assert!(obj.remove(field).is_some(), "{field} is not a config field");
        }
        let parsed: WorldConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.max_move_distance, 1);
    }

    /// Defaulting missing fields must not also swallow misspelled ones: a
    /// typo would otherwise silently run the default.
    #[test]
    fn a_misspelled_config_field_is_an_error() {
        let typo = serde_json::from_str::<WorldConfig>(r#"{ "grid_widht": 64 }"#);
        assert!(typo.is_err());
    }
}
