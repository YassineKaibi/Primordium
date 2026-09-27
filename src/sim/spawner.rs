// @veridikt
// kind: module
// name: Spawner
// purpose: "Initial world seeding: generate viable starting genomes, size their starting energy, and place founder cells per the configured strategy"
// owner: "primordium-maintainers"
// because: "Seeds are biased toward at least one viable acquisition gene and clusters share a mutated ancestor, so the run starts from survivable, locally-related founders rather than mostly-dead noise"
// depends_on: Genome, Energy, World

// Initial seeding strategies, cell creation

use rand::Rng;

use crate::config::{SeedStrategy, WorldConfig};
use crate::sim::cell::Cell;
use crate::sim::genome::{
    self, BASE_GENE_COUNT, GENOME_LEN, Genome, PHOTOSYNTHESIS_RATE, PREDATION_EFFICIENCY,
    SCAVENGE_ABILITY, THERMOSYNTHESIS_RATE,
};
use crate::sim::phase::TriggerCondition;
use crate::sim::world::World;

/// The four energy acquisition gene indices.
const ACQUISITION_GENES: [usize; 4] = [
    PHOTOSYNTHESIS_RATE,
    THERMOSYNTHESIS_RATE,
    SCAVENGE_ABILITY,
    PREDATION_EFFICIENCY,
];

// ── Biased random genome ──────────────────────────────────────────

/// Generate a random genome with a viability floor on acquisition genes.
///
/// All 64 bytes are rolled uniformly. Then, if no acquisition gene meets
/// `min_viable_acquisition`, the highest one is boosted to the floor.
/// Setting the floor to 0 disables this adjustment entirely.

// @veridikt
// purpose: "Roll a random 64-byte genome but guarantee at least one acquisition gene clears the viability floor"
// because: "Pure-random genomes mostly can't feed themselves and die in a tick; the floor keeps the founding population alive long enough for selection to have something to act on"
pub fn random_genome(rng: &mut impl Rng, config: &WorldConfig) -> Genome {
    let mut data = [0u8; GENOME_LEN];
    rng.fill(&mut data[..]);

    let floor = config.min_viable_acquisition;
    if floor == 0 {
        return Genome::new(data);
    }

    // The floor is a promise about what the cell can *express*, not about
    // what byte it carries (G1). Checking the raw byte left 41.7% of founders
    // decoding below the floor: top-N gating attenuates whatever is not among
    // a genome's dozen strongest genes, so a "guaranteed" acquisition gene
    // sitting exactly at the floor is routinely displaced by random parameter
    // genes and comes out 10x smaller.
    //
    // What is guaranteed here is that the gene survives gating — that it is
    // among the `top_n_gene_count` strongest decoded genes. Antagonistic
    // pairs may still cut it down afterwards, and they should.
    let target = floor as f32 / 255.0;
    let expresses = |data: &[u8; GENOME_LEN]| {
        let decoded = Genome::new(*data).decode(config);
        let best = ACQUISITION_GENES
            .iter()
            .map(|&i| decoded.get(i))
            .fold(0.0_f32, f32::max);
        if best < target {
            return false;
        }
        // Rank: fewer than N genes decoding strictly higher means the gene is
        // inside the expressed set rather than under the falloff.
        (0..BASE_GENE_COUNT)
            .filter(|&i| decoded.get(i) > best)
            .count()
            < config.top_n_gene_count as usize
    };

    if expresses(&data) {
        return Genome::new(data);
    }

    // Try each acquisition gene in descending raw order. Which one can carry
    // the cell depends on the rest of the roll, and `spec.md` is explicit
    // that the roll is what decides: "a cell might be a photosynthesizer, a
    // scavenger, or a predator depending on which gene happened to be
    // highest". A genome that rolled high `speed` simply cannot be a
    // photosynthesizer — the two are an antagonistic pair — but it makes a
    // perfectly good hunter.
    let mut candidates = ACQUISITION_GENES;
    candidates.sort_by_key(|&i| std::cmp::Reverse(data[i]));

    for &candidate in &candidates {
        let mut attempt = data;
        // Cut the rivals back to the floor. photosynthesis and
        // thermosynthesis are themselves an antagonistic pair ("distinct
        // energy strategies; investing in both penalizes each"), so a roll
        // high in both expresses neither: photo 255 with thermo 183 decodes
        // to 0.392 before gating and 0.039 after.
        for &other in &ACQUISITION_GENES {
            if other != candidate {
                attempt[other] = attempt[other].min(floor);
            }
        }
        // Cut the candidate's *other* antagonists too, for the same reason.
        // This matters most for photosynthesis, which is the only acquisition
        // gene paired with a non-acquisition one (`speed`, "plants don't
        // run"). Without it a roll that wanted to be a photosynthesizer but
        // rolled high speed was quietly turned into a scavenger instead, and
        // since photosynthesis is the only large renewable niche, seeding
        // drifted away from the one strategy the world can actually support:
        // on seed 2 of `default.json` the founder population went from
        // photosynthesizer-bearing to **zero photosynthesizers**, and the run
        // ended at 17 cells against a recorded 1377.
        for &(a, b, _) in &genome::ANTAGONISTIC_PAIRS {
            let partner = if a == candidate {
                b
            } else if b == candidate {
                a
            } else {
                continue;
            };
            if !ACQUISITION_GENES.contains(&partner) {
                attempt[partner] = attempt[partner].min(floor);
            }
        }
        let start = attempt[candidate];
        for byte in (start..=u8::MAX).step_by(FLOOR_SEARCH_STEP) {
            attempt[candidate] = byte;
            if expresses(&attempt) {
                return Genome::new(attempt);
            }
        }
    }

    // No acquisition gene can clear the falloff against this roll's parameter
    // genes. Max the strongest one and let selection settle it.
    data[candidates[0]] = u8::MAX;
    Genome::new(data)
}

/// Byte step used when raising an acquisition gene to the viability floor.
/// Coarse on purpose — seeding runs once, but it runs over every founder.
const FLOOR_SEARCH_STEP: usize = 8;

// ── Starting energy ───────────────────────────────────────────────

/// Calculate starting energy scaled to genome viability.
///
/// `viability = max(effective acquisition genes) - metabolic_cost`
/// `starting_energy = base + (viability / max_viability) * bonus`
///
/// The `max_viability` denominator is 1.0 (a perfect single-gene
/// specialist with zero cost). Viability is clamped to [0, 1].

// @veridikt
// purpose: "Scale a founder's starting energy by how viable its genome looks (best acquisition gene minus normalized metabolic cost)"
// triggers: Genome.decode, Energy.metabolic_cost
// because: "Giving fitter-looking genomes a bigger head start, instead of a flat handout, biases the opening of the run toward lineages that can actually sustain themselves"
pub fn starting_energy(genome: &Genome, config: &WorldConfig) -> f32 {
    let decoded = genome.decode(config);

    let max_acquisition = ACQUISITION_GENES
        .iter()
        .map(|&i| decoded.get(i))
        .fold(0.0_f32, f32::max);

    let cost = crate::sim::energy::metabolic_cost(&decoded, 128, config);

    // Normalize cost to the same 0..1 scale as acquisition genes.
    // 46 genes each contributing gene^exponent at max=1.0, times the scale.
    let max_possible_cost = crate::sim::genome::BASE_GENE_COUNT as f32
        * 1.0_f32.powf(config.metabolic_cost_exponent)
        * config.metabolic_cost_scale;
    let cost_norm = (cost / max_possible_cost).min(1.0);

    let viability = (max_acquisition - cost_norm).clamp(0.0, 1.0);

    let energy = config.base_spawn_energy + viability * config.bonus_spawn_energy;
    // Anything above the cell's own storage cap is destroyed on tick 1, so
    // handing out more than the cap just inflates the opening population.
    energy.min(crate::sim::energy::storage_cap(&decoded, config))
}

// ── Random uniform strategy ───────────────────────────────────────

/// Scatter `initial_cell_count` cells at unique random positions.
pub fn seed_random_uniform(world: &mut World, config: &WorldConfig, rng: &mut impl Rng) {
    let total_tiles = (config.grid_width * config.grid_height) as usize;
    let count = (config.initial_cell_count as usize).min(total_tiles);

    // Generate unique positions via Fisher-Yates partial shuffle
    let mut indices: Vec<usize> = (0..total_tiles).collect();
    for i in 0..count {
        let j = rng.gen_range(i..total_tiles);
        indices.swap(i, j);
    }

    for &idx in &indices[..count] {
        let x = (idx % config.grid_width as usize) as u16;
        let y = (idx / config.grid_width as usize) as u16;

        let genome = random_genome(rng, config);
        let energy = starting_energy(&genome, config);
        let cell = Cell::new(genome, energy, (x, y));
        let cell_id = world.spawn_cell(cell);
        // Every uniform founder is unrelated to the rest: its own lineage.
        world.record_mut(cell_id).lineage = world.new_lineage();
        world.set_current_tile_cell_id(x, y, cell_id);
    }
}

// ── Random clusters strategy ──────────────────────────────────────

/// Place clusters on a regular grid. Each cluster has one ancestor genome
/// with mutated descendants scattered within a cluster radius.

// @veridikt
// purpose: "Seed founders as genetically-related clusters: one ancestor genome per cluster, with mutated descendants scattered nearby"
// triggers: Genome.mutate, World.spawn_cell, World.new_lineage
// because: "Local genetic relatedness is the precondition for kin behaviors (sharing, packs) to ever get off the ground, so the default strategy plants relatives together"
pub fn seed_random_clusters(world: &mut World, config: &WorldConfig, rng: &mut impl Rng) {
    let cluster_count = config.cluster_count.max(1) as usize;
    let cells_per_cluster = config.initial_cell_count as usize / cluster_count;
    let cluster_radius = config.grid_width as usize / (cluster_count * 2);

    // Grid layout for cluster centers
    let cols = (cluster_count as f64).sqrt().ceil() as usize;
    let rows = cluster_count.div_ceil(cols);
    let col_spacing = config.grid_width as usize / cols;
    let row_spacing = config.grid_height as usize / rows;

    let mut placed = 0;

    for cluster_idx in 0..cluster_count {
        let col = cluster_idx % cols;
        let row = cluster_idx / cols;
        let cx = (col_spacing / 2 + col * col_spacing) as i32;
        // Rows span the full Y axis, edges included. Centring every row
        // inside its band left the bottom row ~49 tiles from the vents, so
        // no founder ever reached the thermal niche and `docs/spec.md`'s
        // "some clusters land near thermal vents at the bottom" was false.
        let cy = if rows > 1 {
            (row * (config.grid_height as usize - 1) / (rows - 1)) as i32
        } else {
            (row_spacing / 2) as i32
        };

        // Generate ancestor genome
        let ancestor = random_genome(rng, config);
        let lineage = world.new_lineage();

        for member in 0..cells_per_cluster {
            if placed >= config.initial_cell_count as usize {
                break;
            }

            let genome = if member == 0 {
                ancestor.clone()
            } else {
                let mut descendant = ancestor.clone();
                descendant.mutate(config, rng);
                descendant
            };

            let Some((x, y)) = find_open_position_in_radius(world, cx, cy, cluster_radius, rng)
            else {
                break; // cluster area is full; the rest of its members are dropped
            };

            let energy = starting_energy(&genome, config);
            let cell = Cell::new(genome, energy, (x, y));
            let cell_id = world.spawn_cell(cell);
            world.record_mut(cell_id).lineage = lineage;
            world.set_current_tile_cell_id(x, y, cell_id);
            placed += 1;
        }
    }
}

/// Find an unoccupied tile within `radius` of (cx, cy), respecting toroidal wrapping.
/// Tries random probing first (fast when sparse), falls back to deterministic scan.
///
/// Returns `None` when every tile in the radius is taken. Returning the centre
/// instead would overwrite the cell already standing there, leaving it alive in
/// the pool on no tile — the same ghost-cell shape as B11.

// @veridikt
// purpose: "Pick a free tile inside a cluster's radius, or report that the cluster area is full"
// because: "Seeding must never write two cells onto one tile: the second overwrites the first's tile_id and the first becomes an unreachable ghost that still eats and still counts as alive"
fn find_open_position_in_radius(
    world: &World,
    cx: i32,
    cy: i32,
    radius: usize,
    rng: &mut impl Rng,
) -> Option<(u16, u16)> {
    let r = radius as i32;

    // Try random offsets first (fast path for sparse clusters)
    for _ in 0..64 {
        let dx = rng.gen_range(-r..=r);
        let dy = rng.gen_range(-r..=r);
        let (x, y) = world.wrap(cx + dx, cy + dy);
        if world.current_tile(x, y).cell_id == 0 {
            return Some((x, y));
        }
    }

    // Fallback: scan all tiles in the radius
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = world.wrap(cx + dx, cy + dy);
            if world.current_tile(x, y).cell_id == 0 {
                return Some((x, y));
            }
        }
    }

    // Cluster area is full.
    None
}

// ── Preset archetypes strategy ────────────────────────────────────

/// The four hand-designed founder species of `SeedStrategy::PresetArchetypes`.
///
/// `docs/spec.md` (Initial Seeding Strategies) names them: photosynthesizer,
/// predator, scavenger, vent-feeder. Unlike `random_clusters`, these are not
/// rolls of the dice — each one is built to be viable in one niche, and the
/// four clusters are placed touching so the trophic links between them can
/// actually fire. The question this answers is whether a food web is
/// *sustainable* here, separately from whether random seeding assembles one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    Photosynthesizer,
    VentFeeder,
    Scavenger,
    Predator,
}

/// All four archetypes, in the order their clusters are laid out.
pub const ARCHETYPES: [Archetype; 4] = [
    Archetype::Photosynthesizer,
    Archetype::VentFeeder,
    Archetype::Scavenger,
    Archetype::Predator,
];

impl Archetype {
    pub fn name(self) -> &'static str {
        match self {
            Archetype::Photosynthesizer => "photosynthesizer",
            Archetype::VentFeeder => "vent_feeder",
            Archetype::Scavenger => "scavenger",
            Archetype::Predator => "predator",
        }
    }
}

/// Byte every gene starts at before the archetype's own genes are written.
///
/// Low on purpose. Top-N gating only lets `top_n_gene_count` genes express,
/// and `metabolic_cost` charges for all 46, so a hand-built specialist that
/// leaves its irrelevant genes near zero both pays less and keeps its slots
/// free for the genes that define it.
const ARCHETYPE_BASELINE: u8 = 6;

/// `photosynthesis_rate` byte for the scavenger: a second income, well below
/// its scavenging so it still classifies (and behaves) as a decomposer.
const SCAVENGER_PHOTOSYNTHESIS: u8 = 120;

/// `reproduction_threshold` byte for the scavenger — lower than the shared
/// 150, because its income arrives in meals rather than every tick.
const SCAVENGER_REPRODUCTION_THRESHOLD: u8 = 90;

/// `armor` byte on the producers. It has to exceed what a *sated*
/// predator can hit for and fall short of what a hungry one can, so that the
/// predator's hunger slot is what decides whether prey dies.
const PREY_ARMOR: u8 = 80;
/// The predator's `attack_power` byte outside its hunger phase: under
/// `PREY_ARMOR`, so a sated predator does no damage at all.
const PREDATOR_BASE_ATTACK: u8 = 110;
/// Threshold byte for the predator's hunger slot: `(188 >> 2) / 63` = 0.746
/// on the EnergyLow strength `1 - energy_fraction`, i.e. it fires below about
/// 25% of the storage cap. No hysteresis (low two bits 0).
const PREDATOR_HUNGER_THRESHOLD: u8 = 188;

/// `aggression_trigger` for the three non-predators: wide enough that all
/// four archetypes read each other as kin (they sit at most 0.125 apart).
const PEACEFUL_AGGRESSION_TRIGGER: u8 = 90;
/// `aggression_trigger` for the predator. Its kin band is
/// `66/255 * (0.5 + 0.5 * precision)` = 0.133: its own lineage stays inside it
/// (under 0.024 after 20 generations of drift) and the other three archetypes
/// (0.159-0.162 away) do not, so it hunts all three. What keeps that from
/// being a plague is the prey's armour and the predator's hunger slot, not
/// who it recognises.
const PREDATORY_AGGRESSION_TRIGGER: u8 = 66;

/// Phase-slot condition byte that makes a slot unfireable: `ThreatNearby`
/// (byte % 8 == 2) whose strength is `n / (n + k)` and so never reaches 1.0.
const PHASE_OFF_CONDITION: u8 = 2;
/// Threshold byte decoding to 1.0 with no hysteresis (`(252 >> 2) / 63`).
const PHASE_OFF_THRESHOLD: u8 = 252;
/// Neutral modifier — `apply_modifier` treats 128 as "no change".
const PHASE_NEUTRAL_MOD: u8 = 128;

/// Build the founder genome for one archetype.
///
/// The phase table is deliberately switched off rather than left random.
/// `spec.md` wants *random* founder phase bytes under the random strategies,
/// because evolution is supposed to clean the table up — but this strategy is
/// the "controlled start", and a food-web measurement should not also be
/// measuring founder phase noise. Mutation reopens the slots over a run.

// @veridikt
// purpose: "Hand-build the 64-byte founder genome for one of the four archetype species"
// because: "Each archetype has to clear its own upkeep in its own niche on tick 1, which means respecting the antagonistic pairs (photosynthesis/speed, attack_power/storage_cap) rather than maxing every gene"
// depends_on: Genome
pub fn archetype_genome(kind: Archetype) -> Genome {
    let mut d = [ARCHETYPE_BASELINE; GENOME_LEN];

    // Shared lifecycle and reproduction policy.
    d[genome::REPRODUCTION_THRESHOLD] = 150;
    d[genome::OFFSPRING_ENERGY_SHARE] = 110;
    d[genome::REPRODUCTION_COOLDOWN] = 20;
    d[genome::MUTATION_RATE] = 128;
    d[genome::MUTATION_MAGNITUDE] = 128;
    d[genome::MAX_AGE] = 96;
    d[genome::MATURITY_AGE] = 24;
    d[genome::MEMBRANE] = 60;
    d[genome::TEMPERATURE_PREFERENCE] = 128;
    // Every archetype is genetically far from the other three, so without a
    // wide kin band each cluster reads its neighbours as threats and the
    // boundary turns into a brawl before any trophic link can pay.
    // `sense` reads both of these raw from the genome (B12), and
    // `effective_trigger = aggression * (0.5 + 0.5 * precision)` is the
    // genetic distance below which a neighbour counts as kin. The four
    // archetypes sit 0.042-0.125 apart, so a producer needs a band wider
    // than that to stay out of a border brawl, while the predator needs one
    // narrower than 0.073 to see the other three as prey and still spare its
    // own cluster.
    d[genome::AGGRESSION_TRIGGER] = PEACEFUL_AGGRESSION_TRIGGER;
    // Prey carry enough armour that a *sated* predator's blow does nothing.
    // Overridden to the baseline for the predator below. See the predator's
    // hunger slot for why this is what makes predation sustainable.
    d[genome::ARMOR] = PREY_ARMOR;

    match kind {
        // Sessile primary producer. speed 0 both because photosynthesis is
        // antagonistic with it and because a producer that walks off its
        // light does worse than one that does not.
        Archetype::Photosynthesizer => {
            d[genome::PHOTOSYNTHESIS_RATE] = 255;
            d[genome::SPEED] = 0;
            d[genome::FLEE_RESPONSE] = 0;
            d[genome::ENERGY_STORAGE_CAP] = 200;
            d[genome::SENSE_RADIUS] = 40;
        }
        // Sessile primary producer at the vents. Same shape, different
        // channel: the vent zone is tiny and its output is split among
        // everyone standing in it, so staying put is the whole strategy.
        Archetype::VentFeeder => {
            d[genome::THERMOSYNTHESIS_RATE] = 255;
            d[genome::SPEED] = 0;
            d[genome::FLEE_RESPONSE] = 0;
            d[genome::ENERGY_STORAGE_CAP] = 200;
            d[genome::SENSE_RADIUS] = 40;
        }
        // Decomposer, built the way selection builds one in this world. A
        // pure, mobile detritivore — the first design here: full sense
        // radius, strong chemotaxis, speed 150 — cannot pay for its own
        // search. Corpses are intermittent, and sense, chemotaxis and speed
        // are all expression cost, so the band sat at break-even with 0-2%
        // ready to reproduce and aged out while uneaten decay piled up around
        // it. The scavengers that thrive under random seeding (2 083 of them
        // on one seed) are nearly sessile (decoded speed 0.01-0.02) and
        // photosynthesize as a second income: a mixotroph that lives partly
        // on light and eats the detritus that falls where it stands.
        // Scavenging stays its strongest channel, so it still classifies as
        // a scavenger.
        //
        // Being sessile it pays nothing for armour (the `armor <-> speed` pair
        // only bites a mover), so it carries the producers' armour and, like
        // them, is safe from a sated predator. A lower reproduction threshold
        // than the producers', because part of its income arrives in meals
        // rather than every tick.
        Archetype::Scavenger => {
            d[genome::SCAVENGE_ABILITY] = 255;
            d[genome::PHOTOSYNTHESIS_RATE] = SCAVENGER_PHOTOSYNTHESIS;
            d[genome::SPEED] = 0;
            d[genome::FLEE_RESPONSE] = 0;
            d[genome::SENSE_RADIUS] = 40;
            d[genome::ENERGY_STORAGE_CAP] = 170;
            d[genome::REPRODUCTION_THRESHOLD] = SCAVENGER_REPRODUCTION_THRESHOLD;
        }
        // Consumer. Three things make it sustainable rather than a plague,
        // each measured (handover, step 12):
        //
        // - **Hunger.** A sated predator must not kill: every kill above its
        //   storage cap is wasted, and the seeded bands were being emptied at
        //   115 kills a tick by predators that could not use the energy. The
        //   base attack (110) does no damage through prey armour; a phase
        //   slot doubles it only when energy is low (below). That cut opening
        //   kills to 13-29 a tick.
        // - **Maturity.** A newborn must not breed at once. With no maturity
        //   every newborn went kill -> breed -> kill, and breeding chains
        //   doubled every ~5 ticks whatever the kill rate. Maxed here, and it
        //   needs `max_maturity_ticks` around 300 to bite.
        // - **Cooldown.** A mature predator breeds at most once per ~100
        //   ticks. Without it, the whole founder cohort matured on the same
        //   tick and bred repeatedly: 249 -> 1130 in 25 ticks.
        //
        // attack_power is antagonistic with energy_storage_cap, so these two
        // are a trade-off rather than two free maxima.
        Archetype::Predator => {
            d[genome::ATTACK_POWER] = PREDATOR_BASE_ATTACK;
            d[genome::PREDATION_EFFICIENCY] = 255;
            d[genome::SPEED] = 170;
            d[genome::CHEMOTAXIS_STRENGTH] = 220;
            d[genome::SENSE_RADIUS] = 255;
            d[genome::FLEE_RESPONSE] = 0;
            d[genome::ENERGY_STORAGE_CAP] = 140;
            d[genome::MATURITY_AGE] = 255;
            d[genome::REPRODUCTION_COOLDOWN] = 255;
            d[genome::ARMOR] = ARCHETYPE_BASELINE;
            d[genome::AGGRESSION_TRIGGER] = PREDATORY_AGGRESSION_TRIGGER;
        }
    }

    // Switch the three phase slots off.
    for slot in 0..genome::PHASE_SLOT_COUNT {
        let base = BASE_GENE_COUNT + slot * genome::PHASE_SLOT_SIZE;
        d[base + genome::PHASE_TRIGGER_CONDITION] = PHASE_OFF_CONDITION;
        d[base + genome::PHASE_TRIGGER_THRESHOLD] = PHASE_OFF_THRESHOLD;
        d[base + genome::PHASE_OFFENSE_MOD] = PHASE_NEUTRAL_MOD;
        d[base + genome::PHASE_DEFENSE_MOD] = PHASE_NEUTRAL_MOD;
        d[base + genome::PHASE_MOBILITY_MOD] = PHASE_NEUTRAL_MOD;
        d[base + genome::PHASE_EFFICIENCY_MOD] = PHASE_NEUTRAL_MOD;
    }

    // ...except the predator's one designed slot: hunger. EnergyLow, doubling
    // offense, firing only when the cell is below ~25% of its cap. Sweeping
    // the threshold, a slot that fired below 60% still out-grazed the prey
    // over ~650 ticks; below ~20% the predators starved out on every seed.
    if kind == Archetype::Predator {
        let base = BASE_GENE_COUNT;
        d[base + genome::PHASE_TRIGGER_CONDITION] = TriggerCondition::EnergyLow as u8;
        d[base + genome::PHASE_TRIGGER_THRESHOLD] = PREDATOR_HUNGER_THRESHOLD;
        d[base + genome::PHASE_OFFENSE_MOD] = 255;
    }

    Genome::new(d)
}

/// How many founders each archetype gets, in `ARCHETYPES` order.
///
/// `config.archetype_population_shares` normalized over
/// `initial_cell_count`. `docs/spec.md` asks for equal populations; the
/// knob exists because equal numbers are what make the start collapse — a
/// predator turns one kill straight into one child, so its numbers respond
/// an order of magnitude faster than a producer's can.
fn archetype_populations(config: &WorldConfig) -> [usize; 4] {
    let shares = config.archetype_population_shares;
    let total: f32 = shares.iter().map(|s| s.max(0.0)).sum();
    if total <= 0.0 {
        let each = config.initial_cell_count as usize / ARCHETYPES.len();
        return [each; 4];
    }
    let n = config.initial_cell_count as f32;
    let mut out = [0usize; 4];
    for (slot, share) in out.iter_mut().zip(shares.iter()) {
        *slot = (n * share.max(0.0) / total) as usize;
    }
    out
}

/// Depth of one archetype band, in rows.
///
/// `config.archetype_band_depth` overrides it; `0` sizes each band so its
/// own founder population fills about half its tiles.
///
/// Bands are shallow on purpose. An occupied tile adds 0.2 to the
/// Beer-Lambert column absorption (`world::tile_absorption`), i.e. it takes
/// 18% of the light away from everything below it, so a producer colony that
/// is deep in `y` shades itself to death: the first layout tried here was a
/// 51x51 square block and measured `sun_used` 15.6 of 255 with photosynthesis
/// paying 0.24 against an upkeep of 0.82.
fn archetype_band_depth(config: &WorldConfig, band_cells: usize) -> usize {
    if config.archetype_band_depth > 0 {
        return config.archetype_band_depth as usize;
    }
    let rows = (band_cells as f32 / (config.grid_width as f32 * 0.5)).ceil() as usize;
    rows.clamp(1, config.grid_height as usize / 8)
}

/// The `(top_row, depth)` of each archetype's band, in `ARCHETYPES` order.
///
/// The four bands span the full width of the grid and stack around the
/// wrap seam, which is where this world's two energy sources meet: light
/// enters at `y = 0` and is absorbed downward, while the vents sit on the
/// bottom row and reach `vent_radius` in every direction, so the vent zone
/// already straddles `y = 0`.
///
/// ```text
///   y = 0                 photosynthesizer   brightest rows, nothing above to shade them
///     (same rows)         scavenger          interleaved: needs light, eats where producers die
///   below it              predator           in contact with the producers it eats
///   ...
///   y = H - depth         vent-feeder        the vent row, adjacent to the photic band
/// ```
///
/// `docs/spec.md` asks for "predefined cluster centers, one per archetype";
/// a full-width band is what that becomes once the light column and the vent
/// row are both taken into account, and it gives every pair of neighbours a
/// `grid_width`-long interface instead of a single corner.

// @veridikt
// purpose: "Stack the four archetype bands around the wrap seam so each sits in the niche it was built for and touches the next along the full width of the grid"
// because: "Light is absorbed down the column and the vents sit on the bottom row, so a producer must be shallow and near y=0 while a vent-feeder must be on the bottom row — the only arrangement that gives all four a live niche and a shared border"
fn archetype_bands(config: &WorldConfig, populations: &[usize; 4]) -> [(i32, usize); 4] {
    let height = config.grid_height as i32;
    let depths: Vec<usize> = populations
        .iter()
        .map(|&n| archetype_band_depth(config, n))
        .collect();
    let (photo, vent, scav, predator) = (depths[0], depths[1], depths[2], depths[3]);
    // The scavenger shares the photic band with the photosynthesizers,
    // interleaved. It is a mixotroph (see `archetype_genome`) and needs the
    // light; in its own band beneath them it sat in their shade and earned
    // 0.17 a tick against an upkeep of 0.88. It is also where producers die
    // of old age, so the detritus it lives on falls next to it.
    let lit = photo + scav;
    [
        (0, lit),                     // photosynthesizer
        (height - vent as i32, vent), // vent-feeder, on the vent row
        (0, lit),                     // scavenger, interleaved with them
        (lit as i32, predator),       // predator, directly below
    ]
}

/// Seed one hand-designed band per archetype, stacked in contact.
///
/// The first member of each band is the archetype genome itself; the rest
/// are mutated copies, the same ancestor-plus-mutation shape
/// `random_clusters` uses.

// @veridikt
// purpose: "Seed the four hand-designed archetype species as full-width bands stacked in contact around the wrap seam"
// triggers: Genome.mutate, World.spawn_cell, World.new_lineage
// because: "A controlled start with the trophic links already in contact is the only way to measure whether a food web is sustainable, separately from whether random founders ever assemble one"
// depends_on: Genome, Energy, World
pub fn seed_preset_archetypes(world: &mut World, config: &WorldConfig, rng: &mut impl Rng) {
    let populations = archetype_populations(config);
    let bands = archetype_bands(config, &populations);

    for ((kind, &(top, depth)), &count) in
        ARCHETYPES.iter().zip(bands.iter()).zip(populations.iter())
    {
        let ancestor = archetype_genome(*kind);
        // One lineage per band, so lineages 1-4 are the archetypes in
        // `ARCHETYPES` order.
        let lineage = world.new_lineage();

        for member in 0..count {
            let genome = if member == 0 {
                ancestor.clone()
            } else {
                let mut descendant = ancestor.clone();
                descendant.mutate(config, rng);
                descendant
            };

            let Some((x, y)) = find_open_position_in_band(world, config, top, depth, rng) else {
                break; // the band is full
            };

            let energy = starting_energy(&genome, config);
            let cell = Cell::new(genome, energy, (x, y));
            let cell_id = world.spawn_cell(cell);
            world.record_mut(cell_id).lineage = lineage;
            world.set_current_tile_cell_id(x, y, cell_id);
        }
    }
}

/// The `(top_row, depth)` band `kind` is seeded into under `config` — where
/// that species lives. With a share of 0 the band is still one row deep, so
/// an archetype left out of the founders still has a place to be introduced.
pub fn archetype_band_rows(config: &WorldConfig, kind: Archetype) -> (i32, usize) {
    let populations = archetype_populations(config);
    let bands = archetype_bands(config, &populations);
    let slot = ARCHETYPES.iter().position(|&k| k == kind).unwrap_or(0);
    bands[slot]
}

/// Introduce `count` cells into a running world as one new lineage: the
/// first an exact copy of `genome`, the rest mutated copies, the same shape a
/// founder band has. They go onto empty tiles of the current grid in the
/// full-width band `[top, top + depth)`, and the world's own RNG places them.
///
/// This is how the lab measures whether a strategy can grow from rare
/// against a settled community. Returns `(lineage, cells placed)`; fewer
/// than `count` are placed when the band runs out of room.

// @veridikt
// purpose: "Place a batch of new cells, tagged as one fresh lineage, into empty tiles of a band in a running world"
// triggers: Genome.mutate, World.spawn_cell, World.new_lineage
// because: "Invasion from rare is the standard test for whether a strategy has a niche; it has to use the run's single RNG so the experiment stays reproducible from the seed"
pub fn inject(
    world: &mut World,
    config: &WorldConfig,
    rng: &mut impl Rng,
    genome: &Genome,
    count: usize,
    top: i32,
    depth: usize,
) -> (u32, usize) {
    let lineage = world.new_lineage();
    let mut placed = 0;
    for member in 0..count {
        let Some((x, y)) = find_open_position_in_band(world, config, top, depth.max(1), rng) else {
            break;
        };
        let mut g = genome.clone();
        if member > 0 {
            g.mutate(config, rng);
        }
        let energy = starting_energy(&g, config);
        let cell_id = world.spawn_cell(Cell::new(g, energy, (x, y)));
        world.record_mut(cell_id).lineage = lineage;
        world.set_current_tile_cell_id(x, y, cell_id);
        placed += 1;
    }
    (lineage, placed)
}

/// Find an unoccupied tile in the full-width band `[top, top + depth)`.
///
/// Returns `None` when the band is full, for the same reason
/// `find_open_position_in_radius` does: writing a second cell onto an
/// occupied tile leaves the first alive in the pool on no tile.
fn find_open_position_in_band(
    world: &World,
    config: &WorldConfig,
    top: i32,
    depth: usize,
    rng: &mut impl Rng,
) -> Option<(u16, u16)> {
    // Random probing first, which is the fast path while the band is sparse.
    for _ in 0..64 {
        let x = rng.gen_range(0..config.grid_width as i32);
        let y = top + rng.gen_range(0..depth as i32);
        let (x, y) = world.wrap(x, y);
        if world.current_tile(x, y).cell_id == 0 {
            return Some((x, y));
        }
    }

    for dy in 0..depth as i32 {
        for x in 0..config.grid_width as i32 {
            let (x, y) = world.wrap(x, top + dy);
            if world.current_tile(x, y).cell_id == 0 {
                return Some((x, y));
            }
        }
    }

    None
}

// ── Dispatcher ────────────────────────────────────────────────────

/// Seed the world with initial cells according to the configured strategy.

// @veridikt
// purpose: "Entry point for seeding: dispatch to the configured SeedStrategy to populate the fresh world"
// because: "The three strategies trade chaos for control — uniform scatter, related-but-random clusters, or the hand-designed archetypes that start a food web already in contact"
pub fn seed_world(world: &mut World, config: &WorldConfig, rng: &mut impl Rng) {
    match config.initial_genome_strategy {
        SeedStrategy::RandomUniform => seed_random_uniform(world, config, rng),
        SeedStrategy::RandomClusters => seed_random_clusters(world, config, rng),
        SeedStrategy::PresetArchetypes => seed_preset_archetypes(world, config, rng),
    }
}

// ── Tests ──────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn default_config() -> WorldConfig {
        WorldConfig::default()
    }

    fn small_config() -> WorldConfig {
        WorldConfig {
            grid_width: 32,
            grid_height: 32,
            initial_cell_count: 50,
            cluster_count: 4,
            min_viable_acquisition: 40,
            ..WorldConfig::default()
        }
    }

    #[test]
    fn spawn_energy_never_exceeds_storage_cap() {
        // Energy handed out above the cap is destroyed on tick 1 (B6).
        let config = default_config();
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        for _ in 0..500 {
            let genome = random_genome(&mut rng, &config);
            let cap = crate::sim::energy::storage_cap(&genome.decode(&config), &config);
            let energy = starting_energy(&genome, &config);
            assert!(
                energy <= cap + f32::EPSILON,
                "spawn energy {energy} exceeds cap {cap}"
            );
        }
    }

    // ── preset_archetypes tests ────────────────────────────────

    fn archetype_config() -> WorldConfig {
        WorldConfig {
            grid_width: 256,
            grid_height: 256,
            initial_cell_count: 400,
            initial_genome_strategy: SeedStrategy::PresetArchetypes,
            ..WorldConfig::default()
        }
    }

    #[test]
    fn every_archetype_clears_its_own_upkeep_in_its_own_niche() {
        // The point of a hand-designed founder is that it is viable on tick
        // 1. A random founder is not: `--niche` measured mean decoded photo
        // at spawn of 0.032 against a metabolism of ~1.9.
        let config = WorldConfig::default();
        for kind in ARCHETYPES {
            let decoded = archetype_genome(kind).decode(&config);
            let cost = crate::sim::energy::metabolic_cost(&decoded, 128, &config);
            let income = match kind {
                Archetype::Photosynthesizer => crate::sim::energy::photo_income(
                    decoded.get(genome::PHOTOSYNTHESIS_RATE),
                    // The producers sit well below the photic top rows.
                    104,
                    &config,
                ),
                // Forty cells sharing one vent. vent_output is split among
                // everyone standing in the zone, so this is a density, not a
                // rate: break-even sits just under 48 cells per vent, and the
                // vent zone holds 169 tiles. The thermal niche is capped by
                // crowding long before it runs out of room.
                Archetype::VentFeeder => crate::sim::energy::thermo_income(
                    decoded.get(genome::THERMOSYNTHESIS_RATE),
                    config.vent_output,
                    40,
                    0,
                    config.vent_cycle,
                ),
                // One fresh corpse: config.corpse_biomass.
                Archetype::Scavenger => {
                    crate::sim::energy::scavenge_income(
                        decoded.get(genome::SCAVENGE_ABILITY),
                        config.corpse_biomass,
                        &config,
                    )
                    .0
                }
                // A predator earns nothing per tick; it earns per kill.
                // Break-even is one kill per `cap / cost` ticks.
                Archetype::Predator => {
                    let cap = crate::sim::energy::storage_cap(&decoded, &config);
                    assert!(
                        cap / cost > 50.0,
                        "predator banks only {:.0} ticks of upkeep per kill",
                        cap / cost
                    );
                    // Sated, it must do no damage at all through prey
                    // armour; hungry, its phase slot must let it kill.
                    let prey = archetype_genome(Archetype::Photosynthesizer).decode(&config);
                    let armor = prey.get(genome::ARMOR) * 255.0;
                    let sated = (decoded.get(genome::ATTACK_POWER) * 255.0 - armor).max(0.0);
                    assert_eq!(sated, 0.0, "a sated predator still deals {sated:.0} damage");

                    let genome = archetype_genome(Archetype::Predator);
                    let mut hungry = decoded.clone();
                    crate::sim::phase::apply_phase_modifiers(&mut hungry, &genome, 1);
                    let bite = (hungry.get(genome::ATTACK_POWER) * 255.0 - armor).max(0.0);
                    assert!(bite > 30.0, "a hungry predator only deals {bite:.0} damage");
                    continue;
                }
            };
            assert!(
                income > cost,
                "{} earns {income:.3} against upkeep {cost:.3}",
                kind.name()
            );
        }
    }

    #[test]
    fn the_predator_is_hungry_only_when_it_is_actually_low() {
        // The hunger slot has to be off when the predator is fed and on when
        // it is starving, or it is either a plague or starves out.
        use crate::sim::phase::{PhaseInput, evaluate_phase};
        let genome = archetype_genome(Archetype::Predator);
        let at = |energy_fraction: f32| {
            let input = PhaseInput {
                energy_fraction,
                threat_count: 0,
                kin_count: 0,
                age: 0,
                food_nearby: true,
                neighbor_count: 0,
                ticks_since_damage: u32::MAX,
                sense_radius: 4,
                maturity_threshold: 1,
                memory_length: 0,
            };
            evaluate_phase(&genome, &input, 0)
        };
        assert_eq!(at(0.9), 0, "a well-fed predator is in its hunger phase");
        assert_eq!(at(0.5), 0, "a half-fed predator is in its hunger phase");
        assert_eq!(at(0.1), 1, "a starving predator is not hungry");
    }

    #[test]
    fn a_predator_reads_the_other_archetypes_as_prey_but_not_its_own_kin() {
        // `sense` classifies a neighbour as kin when genetic_distance <
        // aggression_trigger * (0.5 + 0.5 * precision), both read raw. If
        // the predator's band covers its prey it has no target at all; if the
        // others' bands are too narrow, every border turns into a brawl.
        let trigger = |kind: Archetype| {
            let g = archetype_genome(kind);
            let aggression = g.gene(genome::AGGRESSION_TRIGGER) as f32 / 255.0;
            let precision = g.gene(genome::KIN_RECOGNITION_PRECISION) as f32 / 255.0;
            aggression * (0.5 + 0.5 * precision)
        };
        let predator = archetype_genome(Archetype::Predator);
        let band = trigger(Archetype::Predator);
        for kind in ARCHETYPES {
            let dist = crate::sim::actions::genetic_distance(&predator, &archetype_genome(kind));
            if kind == Archetype::Predator {
                assert!(dist < band, "a predator reads itself as prey");
                continue;
            }
            assert!(
                dist >= band,
                "{} is inside the predator's kin band ({dist:.3} < {band:.3})",
                kind.name()
            );
            assert!(
                dist < trigger(kind),
                "{} reads the predator as a threat",
                kind.name()
            );
        }
        // ...and its own lineage stays kin under realistic drift.
        let config = WorldConfig::default();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for _ in 0..100 {
            let mut child = predator.clone();
            for _ in 0..20 {
                child.mutate(&config, &mut rng);
            }
            let drift = crate::sim::actions::genetic_distance(&predator, &child);
            assert!(
                drift < band,
                "20 generations of drift reads as prey ({drift:.3})"
            );
        }
    }

    #[test]
    fn archetype_bands_stack_in_contact_in_the_niches_they_were_built_for() {
        // The whole point of this strategy: random clusters sit 128 tiles
        // apart and never meet, so nothing ever eats anything.
        let config = archetype_config();
        let populations = archetype_populations(&config);
        let bands = archetype_bands(&config, &populations);
        let [(photo, dp), (vent, dv), (scav, ds), (predator, dh)] = bands;
        let height = config.grid_height as i32;

        // Producers first: the photosynthesizers own the top of the light
        // column, with nothing above them to shade them out.
        assert_eq!(photo, 0);
        // The vent-feeders own the vent row, and their band lies inside the
        // vents' reach.
        assert_eq!(vent + dv as i32, height);
        assert!(
            dv as u32 <= config.vent_radius + 1,
            "vent band is {dv} deep"
        );

        // The scavenger shares the photic rows (it is a mixotroph and starved
        // in their shade when placed beneath them).
        assert_eq!((scav, ds), (photo, dp));
        // The predator sits directly below, in contact with its prey, and
        // the vent-feeders touch the photic band across the wrap seam.
        assert_eq!(predator, photo + dp as i32);
        assert_eq!((vent + dv as i32) % height, photo);
        assert!(
            predator + dh as i32 <= vent,
            "the predator band runs into the vents"
        );
    }

    #[test]
    fn population_shares_size_both_the_founder_counts_and_the_bands() {
        // Equal populations are what spec.md asks for, but they are also
        // what makes the start collapse: a predator turns one kill straight
        // into one child. The shares knob is how a trophic pyramid is
        // measured against that.
        let config = WorldConfig {
            archetype_population_shares: [0.6, 0.2, 0.15, 0.05],
            ..archetype_config()
        };
        let populations = archetype_populations(&config);
        assert_eq!(populations[0], 240); // 0.6 of 400
        assert_eq!(populations[3], 20); // 0.05 of 400
        let bands = archetype_bands(&config, &populations);
        assert!(
            bands[0].1 > bands[3].1,
            "the larger population should get the deeper band: {:?}",
            bands.map(|b| b.1)
        );

        let mut world = World::new(&config);
        seed_world(&mut world, &config, &mut ChaCha8Rng::seed_from_u64(7));
        assert_eq!(
            world.population() as usize,
            populations.iter().sum::<usize>()
        );
    }

    #[test]
    fn preset_archetypes_seeds_all_four_species_on_unique_tiles() {
        let config = archetype_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(11);
        seed_world(&mut world, &config, &mut rng);

        assert_eq!(world.population(), config.initial_cell_count);

        // No two founders on one tile: a duplicate would leave the first
        // alive in the pool on no tile (the B11 ghost shape).
        let occupied: usize = (0..config.grid_width)
            .flat_map(|x| (0..config.grid_height).map(move |y| (x as u16, y as u16)))
            .filter(|&(x, y)| world.current_tile(x, y).cell_id != 0)
            .count();
        assert_eq!(occupied, world.population() as usize);

        // Each archetype's ancestor is present, and the four founder
        // populations are roughly equal (spec.md: "equal population per
        // cluster").
        let populations = archetype_populations(&config);
        for (kind, &per_cluster) in ARCHETYPES.iter().zip(populations.iter()) {
            let ancestor = archetype_genome(*kind);
            let near = world
                .cell_ids()
                .iter()
                .filter(|&&id| {
                    crate::sim::actions::genetic_distance(&world.get_cell(id).genome, &ancestor)
                        < 0.02
                })
                .count();
            assert!(
                near >= per_cluster / 2,
                "only {near} of {per_cluster} founders resemble the {} ancestor",
                kind.name()
            );
        }
    }

    #[test]
    fn a_full_band_drops_its_surplus_instead_of_stacking_cells() {
        // find_open_position_in_radius used to return the cluster centre
        // when the area was full. The caller then wrote a second cell_id
        // onto an occupied tile, leaving the first cell alive in the pool
        // but on no tile — a ghost that still eats and still counts.
        let config = WorldConfig {
            grid_width: 64,
            grid_height: 64,
            initial_cell_count: 400,
            // 128 tiles per band, 100 cells each: comfortably too small
            // once the first band has taken its share.
            archetype_band_depth: 1,
            initial_genome_strategy: SeedStrategy::PresetArchetypes,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        seed_world(&mut world, &config, &mut rng);

        let occupied: usize = (0..config.grid_width)
            .flat_map(|x| (0..config.grid_height).map(move |y| (x as u16, y as u16)))
            .filter(|&(x, y)| world.current_tile(x, y).cell_id != 0)
            .count();
        assert!(world.population() > 0);
        assert_eq!(
            occupied,
            world.population() as usize,
            "seeding stacked cells onto occupied tiles"
        );
        assert!(
            world.population() < config.initial_cell_count,
            "the clusters are too small to hold every founder, so some must be dropped"
        );
    }

    #[test]
    fn preset_archetypes_is_deterministic() {
        let config = archetype_config();
        let mut a = World::new(&config);
        seed_world(&mut a, &config, &mut ChaCha8Rng::seed_from_u64(42));
        let mut b = World::new(&config);
        seed_world(&mut b, &config, &mut ChaCha8Rng::seed_from_u64(42));

        assert_eq!(a.population(), b.population());
        for id in a.cell_ids() {
            assert_eq!(a.get_cell(id).position, b.get_cell(id).position);
            assert_eq!(a.get_cell(id).genome.data, b.get_cell(id).genome.data);
        }
    }

    #[test]
    fn archetype_phase_slots_start_switched_off_except_the_predators_hunger() {
        // spec.md wants *random* founder phase bytes under the random
        // strategies. This one is the controlled start: a food-web
        // measurement should not also be measuring founder phase noise.
        use crate::sim::phase::{PhaseInput, evaluate_phase};
        for kind in ARCHETYPES {
            if kind == Archetype::Predator {
                continue; // covered by the_predator_is_hungry_only_when_it_is_actually_low
            }
            let g = archetype_genome(kind);
            // Every condition pushed to its strongest reading at once.
            let input = PhaseInput {
                energy_fraction: 1.0,
                threat_count: 400,
                kin_count: 400,
                age: 100_000,
                food_nearby: false,
                neighbor_count: 400,
                ticks_since_damage: 0,
                sense_radius: 4,
                maturity_threshold: 1,
                memory_length: 10,
            };
            assert_eq!(
                evaluate_phase(&g, &input, 0),
                0,
                "{} starts in a non-default phase",
                kind.name()
            );
        }
    }

    #[test]
    fn cluster_rows_reach_the_vents() {
        // docs/spec.md: "some clusters land near thermal vents at the
        // bottom". Centring each row inside its band left every founder
        // ~49 tiles away, so the thermal niche was never colonised.
        let config = WorldConfig {
            grid_width: 128,
            grid_height: 128,
            initial_cell_count: 400,
            cluster_count: 16,
            vent_count: 4,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        seed_random_clusters(&mut world, &config, &mut rng);

        let bottom = config.grid_height as i32 - 1;
        let nearest = world
            .cell_ids()
            .iter()
            .map(|&id| (world.get_cell(id).position.1 as i32 - bottom).abs())
            .min()
            .expect("cells were seeded");
        assert!(
            nearest <= config.vent_radius as i32,
            "nearest founder is {nearest} rows from the vents"
        );
    }

    // ── random_genome tests ────────────────────────────────────────

    #[test]
    fn the_viability_floor_holds_after_decode_not_just_in_the_raw_byte() {
        // G1. The floor promises "every cell has at least one working energy
        // acquisition method", but it was checked against the raw byte,
        // before top-N gating. Measured on the old code: 41.7% of founders
        // decoded below the floor, because random parameter genes outrank a
        // gene sitting exactly at it and push it through the x0.1 falloff.
        let config = WorldConfig::default();
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        let target = config.min_viable_acquisition as f32 / 255.0;

        let mut below = 0;
        let n = 3000;
        for _ in 0..n {
            let decoded = random_genome(&mut rng, &config).decode(&config);
            let best = ACQUISITION_GENES
                .iter()
                .map(|&i| decoded.get(i))
                .fold(0.0_f32, f32::max);
            if best < target {
                below += 1;
            }
        }
        assert_eq!(
            below, 0,
            "{below} of {n} founders decode below the viability floor (1251 of 3000 on the old code)"
        );
    }

    #[test]
    fn biased_genome_has_viable_acquisition() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let g = random_genome(
                &mut rng,
                &WorldConfig {
                    min_viable_acquisition: 40,
                    ..WorldConfig::default()
                },
            );
            let max_acq = ACQUISITION_GENES.iter().map(|&i| g.gene(i)).max().unwrap();
            assert!(max_acq >= 40, "max acquisition gene {max_acq} < floor 40");
        }
    }

    #[test]
    fn biased_genome_floor_disabled_when_zero() {
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        // With floor=0, some genomes will naturally have very low acquisition.
        // P(max of 4 uniform bytes < 40) ≈ 0.06%, so we need many trials.
        let mut had_low = false;
        for _ in 0..10_000 {
            let g = random_genome(
                &mut rng,
                &WorldConfig {
                    min_viable_acquisition: 0,
                    ..WorldConfig::default()
                },
            );
            let max_acq = ACQUISITION_GENES.iter().map(|&i| g.gene(i)).max().unwrap();
            if max_acq < 40 {
                had_low = true;
                break;
            }
        }
        assert!(
            had_low,
            "with floor=0, should occasionally produce low-acquisition genomes"
        );
    }

    #[test]
    fn biased_genome_is_deterministic() {
        let g1 = random_genome(
            &mut ChaCha8Rng::seed_from_u64(42),
            &WorldConfig {
                min_viable_acquisition: 40,
                ..WorldConfig::default()
            },
        );
        let g2 = random_genome(
            &mut ChaCha8Rng::seed_from_u64(42),
            &WorldConfig {
                min_viable_acquisition: 40,
                ..WorldConfig::default()
            },
        );
        assert_eq!(g1.data, g2.data);
    }

    // ── starting_energy tests ──────────────────────────────────────

    #[test]
    fn starting_energy_at_least_base() {
        let config = default_config();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..50 {
            let g = random_genome(&mut rng, &config);
            let e = starting_energy(&g, &config);
            assert!(
                e >= config.base_spawn_energy,
                "energy {e} < base {}",
                config.base_spawn_energy
            );
        }
    }

    #[test]
    fn starting_energy_viable_genome_gets_bonus() {
        let config = default_config();
        // Build a "good" genome: high photosynthesis, low everything else
        let mut data = [0u8; GENOME_LEN];
        data[PHOTOSYNTHESIS_RATE] = 255;
        let g = Genome::new(data);
        let e = starting_energy(&g, &config);
        assert!(
            e > config.base_spawn_energy,
            "viable genome should get bonus: energy={e}, base={}",
            config.base_spawn_energy
        );
    }

    #[test]
    fn starting_energy_bad_genome_near_base() {
        let config = default_config();
        // A genome with zero acquisition genes gets no viability bonus
        let g = Genome::new([0u8; GENOME_LEN]);
        let e = starting_energy(&g, &config);
        assert!(
            (e - config.base_spawn_energy).abs() < f32::EPSILON,
            "zero-acquisition genome should get exactly base energy {}, got {e}",
            config.base_spawn_energy
        );
    }

    // ── random_uniform tests ───────────────────────────────────────

    #[test]
    fn uniform_correct_count() {
        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_random_uniform(&mut world, &config, &mut rng);
        assert_eq!(world.population(), config.initial_cell_count);
    }

    #[test]
    fn uniform_no_duplicate_positions() {
        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_random_uniform(&mut world, &config, &mut rng);

        // Count occupied tiles
        let occupied: usize = (0..config.grid_width)
            .flat_map(|x| (0..config.grid_height).map(move |y| (x as u16, y as u16)))
            .filter(|&(x, y)| world.current_tile(x, y).cell_id != 0)
            .count();
        assert_eq!(
            occupied, config.initial_cell_count as usize,
            "occupied tiles should equal cell count (no duplicates)"
        );
    }

    #[test]
    fn uniform_deterministic() {
        let config = small_config();

        let mut w1 = World::new(&config);
        seed_random_uniform(&mut w1, &config, &mut ChaCha8Rng::seed_from_u64(42));

        let mut w2 = World::new(&config);
        seed_random_uniform(&mut w2, &config, &mut ChaCha8Rng::seed_from_u64(42));

        // Same positions should be occupied
        for y in 0..config.grid_height as u16 {
            for x in 0..config.grid_width as u16 {
                assert_eq!(
                    w1.current_tile(x, y).cell_id != 0,
                    w2.current_tile(x, y).cell_id != 0,
                    "tile ({x},{y}) differs between runs"
                );
            }
        }
    }

    // ── random_clusters tests ──────────────────────────────────────

    #[test]
    fn clusters_populate_world() {
        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_random_clusters(&mut world, &config, &mut rng);
        assert!(world.population() > 0, "cluster seeding should place cells");
    }

    #[test]
    fn clusters_no_duplicate_positions() {
        let config = small_config();
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_random_clusters(&mut world, &config, &mut rng);

        let occupied: usize = (0..config.grid_width)
            .flat_map(|x| (0..config.grid_height).map(move |y| (x as u16, y as u16)))
            .filter(|&(x, y)| world.current_tile(x, y).cell_id != 0)
            .count();
        assert_eq!(
            occupied,
            world.population() as usize,
            "each cell should occupy a unique tile"
        );
    }

    #[test]
    fn clusters_genetic_similarity_within_cluster() {
        // With 1 cluster, all cells should be genetically similar
        let config = WorldConfig {
            grid_width: 32,
            grid_height: 32,
            initial_cell_count: 10,
            cluster_count: 1,
            min_viable_acquisition: 40,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_random_clusters(&mut world, &config, &mut rng);

        // Collect all cell genomes
        let mut genomes = Vec::new();
        for y in 0..config.grid_height as u16 {
            for x in 0..config.grid_width as u16 {
                let cid = world.current_tile(x, y).cell_id;
                if cid != 0 {
                    genomes.push(world.get_cell(cid).genome.data);
                }
            }
        }
        assert!(genomes.len() >= 2, "need at least 2 cells");

        // All cells share the same ancestor, so their genomes should be
        // more similar to each other than two fully random genomes would be.
        // Mutation rate/magnitude vary per ancestor, so we measure average
        // byte distance rather than counting changed positions.
        let ancestor = &genomes[0];
        for g in &genomes[1..] {
            let total_dist: u32 = ancestor
                .iter()
                .zip(g.iter())
                .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs() as u32)
                .sum();
            let avg_dist = total_dist as f32 / GENOME_LEN as f32;
            // Two fully random genomes average ~85 distance per byte.
            // Mutated descendants should be noticeably closer.
            assert!(
                avg_dist < 85.0,
                "avg byte distance {avg_dist} — descendants should be closer than random"
            );
        }
    }

    // ── seed_world dispatcher test ─────────────────────────────────

    #[test]
    fn seed_world_dispatches_correctly() {
        let mut config = small_config();
        config.initial_genome_strategy = SeedStrategy::RandomUniform;
        let mut world = World::new(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        seed_world(&mut world, &config, &mut rng);
        assert_eq!(world.population(), config.initial_cell_count);
    }
}
