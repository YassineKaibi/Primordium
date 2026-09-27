// @veridikt
// kind: module
// name: Actions
// purpose: "What a cell perceives and does: sensing, the priority-gated action choice, and the simultaneous resolution of all cells' actions into the next grid"
// owner: "primordium-maintainers"
// because: "decide() is pure and buffered, then resolve_all applies everything in strict global priority order (reproduce>attack>move>share>idle) so the tick is order-independent and deterministic"
// depends_on: World, Genome, Stats

// Action enum (Move, Attack, Reproduce, Share, Idle),
// decision logic, conflict resolution

use crate::config::WorldConfig;
use crate::sim::cell::Cell;
use crate::sim::genome::{self, BASE_GENE_COUNT, DecodeCache, DecodedGenes, Genome};
use crate::sim::stats::{self, DeathCause};
use crate::sim::world::{Tile, World};

// ── Action enum ────────────────────────────────────────────────────

/// An action chosen by a cell during the decision phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Spawn offspring at target tile.
    Reproduce(u16, u16),
    /// Attack target cell by id.
    Attack(u32),
    /// Flee toward target tile (away from threat).
    Flee(u16, u16),
    /// Move to target tile.
    Move(u16, u16),
    /// Share energy with target kin cell by id.
    Share(u32),
    /// Do nothing this tick.
    Idle,
}

impl Action {
    /// Position in the priority order Reproduce > Attack > Flee > Move >
    /// Share > Idle, as `TickStats::actions` indexes it.
    pub fn priority_index(self) -> usize {
        match self {
            Action::Reproduce(..) => 0,
            Action::Attack(_) => 1,
            Action::Flee(..) => 2,
            Action::Move(..) => 3,
            Action::Share(_) => 4,
            Action::Idle => 5,
        }
    }
}

// ── TileSnapshot ───────────────────────────────────────────────────

/// Lightweight Copy snapshot of a tile's environmental state.
/// Used so SenseResult doesn't hold references into the world.
#[derive(Debug, Clone, Copy)]
pub struct TileSnapshot {
    pub sunlight: u8,
    pub temperature: u8,
    pub decay_energy: f32,
    pub toxin: f32,
    pub pheromone: f32,
}

impl TileSnapshot {
    pub fn from_tile(tile: &Tile) -> Self {
        Self {
            sunlight: tile.sunlight,
            temperature: tile.temperature,
            decay_energy: tile.decay_energy,
            toxin: tile.toxin,
            pheromone: tile.pheromone,
        }
    }
}

// ── Genetic distance ───────────────────────────────────────────────

/// Manhattan distance over the 46 base genes, normalized to 0.0-1.0.

// @veridikt
// purpose: "Normalized genetic difference between two genomes; the raw signal behind kin-vs-threat classification"
// because: "Kin recognition compares this distance against an aggression threshold scaled by KIN_RECOGNITION_PRECISION, so 'who is family' is itself an evolvable, fuzzy judgement"
pub fn genetic_distance(a: &Genome, b: &Genome) -> f32 {
    let sum: u32 = (0..BASE_GENE_COUNT)
        .map(|i| (a.gene(i) as i16 - b.gene(i) as i16).unsigned_abs() as u32)
        .sum();
    sum as f32 / (BASE_GENE_COUNT as f32 * 255.0)
}

/// The 8 unit headings, in the order `SenseResult::free_run` is indexed.
pub const HEADINGS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

// ── SenseResult ────────────────────────────────────────────────────

/// A non-kin neighbour and the weapons it would strike with, so the Flee
/// gate can tell a threat that can hurt the cell from one that cannot.
#[derive(Debug, Clone, Copy)]
pub struct Threat {
    pub pos: (u16, u16),
    pub cell_id: u32,
    /// Chebyshev distance from the sensing cell.
    pub dist: u16,
    /// Decoded, phase-modified `attack_power` and `venom`.
    pub attack_power: f32,
    pub venom: f32,
}

/// Aggregated sensing data for one cell's neighborhood scan.
pub struct SenseResult {
    /// Position of nearest food source (depends on cell's strongest acquisition gene).
    pub nearest_food: Option<(u16, u16)>,
    /// Nearest hostile neighbor: (x, y, cell_id, chebyshev_distance).
    pub nearest_threat: Option<(u16, u16, u32, u16)>,
    /// Number of genetically similar neighbors (kin).
    pub kin_count: u32,
    /// Number of hostile neighbors (non-kin).
    pub threat_count: u32,
    /// Nearest kin neighbor: (x, y, cell_id). Used for Share action.
    pub nearest_kin: Option<(u16, u16, u32)>,
    /// Direction toward highest pheromone, weighted by signal_sensitivity.
    pub pheromone_gradient: (i8, i8),
    /// Total neighbors within sense radius.
    pub neighbor_count: u32,
    /// Whether any energy source was detected in radius.
    pub food_nearby: bool,
    /// Environmental snapshot of the cell's own tile.
    pub local_tile: TileSnapshot,
    /// Empty tiles within radius 1 (for placement actions).
    pub empty_adjacent: Vec<(u16, u16)>,
    /// Empty tiles within the cell's `offspring_scatter` reach, for
    /// placing a child. `docs/spec.md` gene 22: "distance from parent at
    /// which offspring spawns", capped by `sense_radius`. Always a superset
    /// of `empty_adjacent`.
    pub empty_scatter: Vec<(u16, u16)>,
    /// For each of the 8 headings in `HEADINGS`, how many consecutive empty
    /// tiles lie in that direction (up to the sense radius). A multi-tile move
    /// can only travel as far as the run is clear.
    pub free_run: [u8; 8],
    /// Decoded, phase-modified armour of `nearest_threat`, scaled to damage
    /// units (x255). Only filled when `attack_only_when_harmful` is on.
    pub nearest_threat_armor: f32,
    /// Every non-kin neighbour with its weapons, in scan order. Only filled
    /// when `flee_can_escape` is on.
    pub threats: Vec<Threat>,
    /// The cell's own tile's share of the food around it, 0..1: its food
    /// value over that plus the richest other food tile in sense range. 1 is
    /// a meal with nothing comparable in sight, 0.5 a tile no better than the
    /// best one nearby, 0 no food underfoot. Always 0 for a hunter.
    pub own_food_share: f32,
    /// Grid dimensions, so headings can use toroidal deltas.
    pub world_size: (u32, u32),
}

// ── Gene value mapping helpers ─────────────────────────────────────

/// Energy a cell must hold before it may reproduce: its own threshold gene
/// applied to its storage cap, but never below `reproduction_energy_floor`.
///
/// The floor is what stops a small-cap cell from splitting at near-zero
/// energy and filling the grid with children too poor to survive a tick.
pub fn mapped_reproduction_threshold(genes: &DecodedGenes, config: &WorldConfig) -> f32 {
    let cap = crate::sim::energy::storage_cap(genes, config);
    (genes.get(genome::REPRODUCTION_THRESHOLD) * cap).max(config.reproduction_energy_floor)
}

/// Ticks before a cell may reproduce, capped at a fraction of its own
/// lifespan.
///
/// Without the cap a lineage whose `maturity_age` outruns its `max_age` is
/// sterile by construction and dies out whatever the environment does. This
/// is the same kind of physical cap as `attack_range` <= `sense_radius`.

// @veridikt
// purpose: "Map the maturity_age gene to ticks and clamp it below the cell's own lifespan"
// depends_on: Energy.lifespan_ticks
pub fn mapped_maturity_age(genes: &DecodedGenes, config: &WorldConfig) -> u32 {
    let raw = (genes.get(genome::MATURITY_AGE) * config.max_maturity_ticks as f32) as u32;
    let lifespan = crate::sim::energy::lifespan_ticks(genes, config);
    let ceiling = (lifespan as f32 * config.maturity_lifespan_fraction) as u32;
    raw.min(ceiling)
}

/// Map attack_range gene to tile distance (1-3).
fn mapped_attack_range(genes: &DecodedGenes) -> u16 {
    let raw = (genes.get(genome::ATTACK_RANGE) * 3.0).ceil() as u16;
    raw.max(1)
}

/// Map territorial_radius to tiles, 0 (defends nothing) up to the cell's own
/// sense radius — it cannot defend what it cannot see.
///
/// `decode` has already capped the gene by `speed` (Physical Caps: "can't
/// patrol unreachable area").
fn mapped_territorial_radius(genes: &DecodedGenes) -> i32 {
    let reach = mapped_sense_radius(genes) as f32;
    (genes.get(genome::TERRITORIAL_RADIUS) * reach).round() as i32
}

/// Map offspring_scatter to a placement radius in tiles, 1 up to the cell's
/// own sense radius.
///
/// `decode` has already capped the gene by `sense_radius` (Physical Caps),
/// so this only has to turn it into tiles.
fn mapped_offspring_scatter(genes: &DecodedGenes) -> i32 {
    let reach = mapped_sense_radius(genes) as f32;
    ((genes.get(genome::OFFSPRING_SCATTER) * reach).ceil() as i32).max(1)
}

/// Share of the parent's energy its child is born with: the
/// `offspring_energy_share` gene, bounded to
/// `[min_offspring_energy_share, max_offspring_energy_share]`.
///
/// Unbounded, a gene decoding to 0 made a child with no energy (a birth, a
/// death at cleanup, and `corpse_biomass` of decay from nothing), and one
/// decoding to 1 left the parent dead in childbirth. Bounding only the exact
/// ends is not enough: a child born with 1% of a parent at the reproduction
/// floor still starves in the tick it is born, and 0.7-2.4% of all births did.
/// Inside the bounds the gene is used as it is.

// @veridikt
// purpose: "Bound the offspring_energy_share gene so both child and parent leave a birth alive"
// because: "Out-of-range shares turned births into corpses: a zero-energy child still left corpse_biomass of decay, energy from nothing, and a share of 1 killed the parent; a clamp leaves every in-range genome untouched"
pub fn offspring_energy_share(genes: &DecodedGenes, config: &WorldConfig) -> f32 {
    // max-then-min rather than `clamp`, which panics on a config whose
    // bounds cross; the ceiling wins then.
    genes
        .get(genome::OFFSPRING_ENERGY_SHARE)
        .max(config.min_offspring_energy_share)
        .min(config.max_offspring_energy_share)
}

/// Map reproduction_cooldown gene to tick count.
pub fn mapped_reproduction_cooldown(genes: &DecodedGenes) -> u16 {
    (genes.get(genome::REPRODUCTION_COOLDOWN) * 100.0) as u16
}

// ── Decision logic ─────────────────────────────────────────────────

/// Choose an action for a cell based on priority gates.
///
/// Priority: Reproduce > Attack > Flee > Move > Share > Idle.
/// First passing gate wins.

// @veridikt
// purpose: "Choose one action for a cell this tick by walking fixed priority gates and taking the first that passes"
// because: "Priority is hardcoded (survival/reproduction beats movement beats altruism) but every gate's condition is gene-driven, so which gate a cell can actually reach is still under genetic control"
// assumes: "genes are already decoded + phase-modified; rng is the seeded stream so tie-breaks and exploratory moves stay reproducible"
pub fn decide(
    cell: &Cell,
    genes: &DecodedGenes,
    sense: &SenseResult,
    config: &WorldConfig,
    rng: &mut impl rand::Rng,
) -> Action {
    // Gate 0: Dormancy. spec.md gene 34 calls this "the dormancy phase" —
    // a cell below its own dormancy_trigger shuts down. `dormancy_cost`
    // buys it a cheaper metabolism in `energy::update_energy`, and the price
    // is paid here: it does nothing at all, so it cannot reproduce, hunt or
    // flee until income lifts it back above the trigger. Without that price
    // a low dormancy_cost would simply make starvation optional.
    let cap = crate::sim::energy::storage_cap(genes, config);
    let energy_fraction = if cap > 0.0 { cell.energy / cap } else { 0.0 };
    if crate::sim::energy::dormancy_multiplier(genes, energy_fraction, config) < 1.0 {
        return Action::Idle;
    }

    // Gate 1: Reproduce
    let repro_threshold = mapped_reproduction_threshold(genes, config);
    let maturity = mapped_maturity_age(genes, config);
    // >= not >: energy is clipped at the storage cap, so a cell whose
    // threshold gene sits at 1.0 would otherwise never qualify.
    if cell.energy >= repro_threshold
        && cell.cooldown_remaining == 0
        && cell.age >= maturity
        && !sense.empty_scatter.is_empty()
    {
        let idx = rng.gen_range(0..sense.empty_scatter.len());
        let (tx, ty) = sense.empty_scatter[idx];
        return Action::Reproduce(tx, ty);
    }

    // Gate 2: Attack
    let attack_range = mapped_attack_range(genes);
    // With `attack_only_when_harmful`, a blow that cannot get through the
    // target's armour is not an attack at all, and the threat falls through
    // to the Flee gate — otherwise harmless prey "fights" an adjacent
    // predator instead of running.
    let can_hurt = !config.attack_only_when_harmful
        || genes.get(genome::ATTACK_POWER) * 255.0 > sense.nearest_threat_armor;
    if let Some((_tx, _ty, target_id, dist)) = sense.nearest_threat
        && dist <= attack_range
        && can_hurt
    {
        return Action::Attack(target_id);
    }

    // Gate 3: Flee. With `flee_can_escape` a flight is a move, so it takes
    // the speed to make it (`speed * flee_response`), and a cell only runs
    // from a threat that can actually hurt it (docs/spec.md gene 10: "away
    // from larger or aggressive neighbors"). Without it, any non-kin in sight
    // sends a cell with any flee_response at all running, every tick.
    let flee_from = if config.flee_can_escape {
        nearest_danger(genes, sense)
            .filter(|_| {
                rng.r#gen::<f32>() < genes.get(genome::SPEED) * genes.get(genome::FLEE_RESPONSE)
            })
            .map(|t| t.pos)
    } else {
        sense
            .nearest_threat
            .filter(|_| genes.get(genome::FLEE_RESPONSE) > 0.0)
            .map(|(x, y, _, _)| (x, y))
    };
    if let Some(threat) = flee_from
        && let Some(flee_tile) = flee_direction(cell, threat, sense)
    {
        let (fx, fy) = extend_move(cell.position, flee_tile, genes, sense, config);
        return Action::Flee(fx, fy);
    }

    // Gate 4: Move
    // Adhesion sticks a cell to its neighbouring kin (spec.md gene 28,
    // "tendency to stick to adjacent genetically similar cells. Enables
    // cluster formation"). The gene was never read, so nothing held a
    // colony together. It scales down the chance of moving in proportion to
    // how much of the neighbourhood is kin.
    let kin_share = if sense.neighbor_count > 0 {
        sense.kin_count as f32 / sense.neighbor_count as f32
    } else {
        0.0
    };
    let stickiness = genes.get(genome::ADHESION) * kin_share;
    let mut move_chance = genes.get(genome::SPEED) * (1.0 - stickiness);
    // With `foragers_stay_on_food` a cell standing on food is held there by
    // its own chemotaxis (spec.md gene 9, "tendency to move toward nearby
    // energy sources": the nearest source is the one underfoot). Without it a
    // mobile scavenger walks off a corpse worth eight meals on its speed roll,
    // steered by chemotaxis toward the nearest *other* food tile, since the
    // food scan skips its own.
    if config.foragers_stay_on_food {
        move_chance *= 1.0 - genes.get(genome::CHEMOTAXIS_STRENGTH) * sense.own_food_share;
    }
    if rng.r#gen::<f32>() < move_chance
        && let Some(step) = compute_move_target(cell, genes, sense, rng)
    {
        let (mx, my) = extend_move(cell.position, step, genes, sense, config);
        return Action::Move(mx, my);
    }

    // Gate 5: Share
    if genes.get(genome::RESOURCE_SHARING) > 0.0 && cell.energy > repro_threshold * 0.5 {
        // Find an adjacent kin to share with (from the neighbor list in sense)
        if let Some(kin_id) = find_adjacent_kin(sense) {
            return Action::Share(kin_id);
        }
    }

    // Gate 6: Idle (always)
    Action::Idle
}

/// The nearest non-kin neighbour whose blow or venom would actually cost
/// this cell energy — judged by the same exchange `resolve_attack` settles,
/// against the cell's own armour and membrane.

// @veridikt
// purpose: "Pick the nearest threat that could hurt this cell, by running each threat's weapons through resolve_attack against the cell's armour and membrane"
// because: "Every non-kin counts as a threat for sensing, but running from one that cannot get through the armour only costs the cell its tick; venom counts because it lands whatever the armour"
fn nearest_danger<'a>(genes: &DecodedGenes, sense: &'a SenseResult) -> Option<&'a Threat> {
    let me = CombatStats {
        cell_id: 0,
        attack_power: genes.get(genome::ATTACK_POWER),
        armor: genes.get(genome::ARMOR),
        venom: genes.get(genome::VENOM),
    };
    let membrane = genes.get(genome::MEMBRANE);
    let mut nearest: Option<&Threat> = None;
    for t in &sense.threats {
        let them = CombatStats {
            cell_id: t.cell_id,
            attack_power: t.attack_power,
            armor: 0.0,
            venom: t.venom,
        };
        let blow = resolve_attack(&them, &me);
        let venom_hurts = blow.venom_ticks > 0
            && crate::sim::energy::venom_tick_damage(blow.venom_damage, membrane) > 0.0;
        if (blow.damage_to_defender > 0.0 || venom_hurts) && nearest.is_none_or(|n| t.dist < n.dist)
        {
            nearest = Some(t);
        }
    }
    nearest
}

/// Find the flee direction: opposite vector from the threat at `(tx, ty)`,
/// resolved to an adjacent tile.
fn flee_direction(cell: &Cell, (tx, ty): (u16, u16), sense: &SenseResult) -> Option<(u16, u16)> {
    let (cx, cy) = cell.position;
    let (width, height) = sense.world_size;
    // Away from the threat, along the shortest toroidal path.
    let ndx = -toroidal_delta(cx, tx, width).signum();
    let ndy = -toroidal_delta(cy, ty, height).signum();

    // Find an empty adjacent tile in that direction (already wrapped).
    sense
        .empty_adjacent
        .iter()
        .copied()
        .find(|&(ex, ey)| {
            let edx = toroidal_delta(cx, ex, width).signum();
            let edy = toroidal_delta(cy, ey, height).signum();
            edx == ndx && edy == ndy
        })
        .or_else(|| {
            // Fallback: any empty tile roughly away from threat
            sense.empty_adjacent.first().copied()
        })
}

/// Find an adjacent kin cell id for sharing.
fn find_adjacent_kin(sense: &SenseResult) -> Option<u32> {
    sense.nearest_kin.map(|(_, _, cell_id)| cell_id)
}

/// Compute the movement target tile by combining directional influences.
///
/// Combines five heading influences (direction_bias, noise, chemotaxis,
/// pack_affinity, memory_dir) into a vector, then picks the empty adjacent
/// tile most aligned with that heading via dot product.

// @veridikt
// purpose: "Pick the move target by summing five gene-weighted heading influences into one vector and choosing the best-aligned empty neighbor"
// because: "Blending bias, noise, chemotaxis, pack pull, and remembered direction into a single heading lets complex movement (hunting gradients, swarming) emerge from a few scalar genes rather than explicit behavior code"
/// Add `weight` worth of the direction (dx, dy) to a heading.
///
/// Normalising matters: a food tile four steps away must not outvote a
/// neighbour one step away just because its delta is longer.
fn add_unit(hx: &mut f32, hy: &mut f32, dx: f32, dy: f32, weight: f32) {
    let len = (dx * dx + dy * dy).sqrt();
    if len > f32::EPSILON && weight != 0.0 {
        *hx += dx / len * weight;
        *hy += dy / len * weight;
    }
}

fn compute_move_target(
    cell: &Cell,
    genes: &DecodedGenes,
    sense: &SenseResult,
    rng: &mut impl rand::Rng,
) -> Option<(u16, u16)> {
    if sense.empty_adjacent.is_empty() {
        return None;
    }

    // 1. Base heading from direction_bias + noise perturbation
    let direction_bias = genes.get(genome::DIRECTION_BIAS);
    let direction_noise = genes.get(genome::DIRECTION_NOISE);
    // Symmetric: a one-sided [0, noise*PI) term rotated every heading the
    // same way, curving every path and drifting whole colonies.
    let noise_angle = (rng.r#gen::<f32>() * 2.0 - 1.0) * direction_noise * std::f32::consts::PI;
    let angle = direction_bias * 2.0 * std::f32::consts::PI;
    let combined_angle = angle + noise_angle;

    let mut hx = combined_angle.cos();
    let mut hy = combined_angle.sin();

    // 2. Chemotaxis: pull toward pheromone gradient
    let cx = cell.position.0;
    let cy = cell.position.1;
    let (width, height) = sense.world_size;

    // Chemotaxis: toward the nearest energy source, per docs/spec.md gene 9.
    // This used to be spent on the pheromone gradient while `nearest_food`
    // went unread, so no cell ever moved toward food.
    // sense_priority splits attention between the two: spec.md gene 24,
    // "0 = food, 255 = threats. Gradient." The gene was never read, so every
    // cell foraged with equal disregard for what was hunting it.
    let priority = genes.get(genome::SENSE_PRIORITY);
    let chemotaxis = genes.get(genome::CHEMOTAXIS_STRENGTH);
    if let Some((fx, fy)) = sense.nearest_food {
        let (dx, dy) = (
            toroidal_delta(cx, fx, width) as f32,
            toroidal_delta(cy, fy, height) as f32,
        );
        add_unit(&mut hx, &mut hy, dx, dy, chemotaxis * (1.0 - priority));
    }
    // The threat half: steer away from what the cell is watching. This is
    // not the Flee gate (which is a separate, higher-priority action); it is
    // the standing bias of a cell that watches threats over food.
    //
    // Unless the intruder is inside the cell's own territory, in which case
    // it steers *toward* it: spec.md gene 41, territorial_radius, "radius of
    // area the cell defends. Attacks non-kin who enter." The gene was never
    // read, so nothing was ever defended.
    if let Some((tx, ty, _, threat_dist)) = sense.nearest_threat {
        let (dx, dy) = (
            toroidal_delta(cx, tx, width) as f32,
            toroidal_delta(cy, ty, height) as f32,
        );
        let territory = mapped_territorial_radius(genes);
        if threat_dist as i32 <= territory {
            add_unit(
                &mut hx,
                &mut hy,
                dx,
                dy,
                genes.get(genome::TERRITORIAL_RADIUS),
            );
        } else {
            add_unit(&mut hx, &mut hy, -dx, -dy, chemotaxis * priority);
        }
    }

    // Pheromone is a separate sense; the gradient is already weighted by
    // signal_sensitivity in `sense`.
    add_unit(
        &mut hx,
        &mut hy,
        sense.pheromone_gradient.0 as f32,
        sense.pheromone_gradient.1 as f32,
        1.0,
    );

    // 3. Pack affinity: pull toward nearest kin
    if let Some(nearest_kin) = sense.nearest_kin
        && nearest_kin.2 != 0
    {
        let pack = genes.get(genome::PACK_AFFINITY);
        let (dx, dy) = (
            toroidal_delta(cx, nearest_kin.0, width) as f32,
            toroidal_delta(cy, nearest_kin.1, height) as f32,
        );
        add_unit(&mut hx, &mut hy, dx, dy, pack);
    }

    // 4. Memory: momentum from the last step this cell took, weighted by
    // memory_length. The field was never written, so this term was always
    // zero and gene 25 did nothing ("0 = purely reactive" was true of every
    // cell). `memory_ticks` is what makes the gene a *duration*.
    if cell.memory_dir != (0, 0) {
        let memory = genes.get(genome::MEMORY_LENGTH);
        add_unit(
            &mut hx,
            &mut hy,
            cell.memory_dir.0 as f32,
            cell.memory_dir.1 as f32,
            memory,
        );
    }

    // 5. Zero-vector fallback: pick random tile
    if hx.abs() < f32::EPSILON && hy.abs() < f32::EPSILON {
        let idx = rng.gen_range(0..sense.empty_adjacent.len());
        return Some(sense.empty_adjacent[idx]);
    }

    // 6. Score each empty adjacent tile by dot product with heading
    let mut best_tile = sense.empty_adjacent[0];
    let mut best_score = f32::NEG_INFINITY;

    for &(tx, ty) in &sense.empty_adjacent {
        let dx = toroidal_delta(cx, tx, width) as f32;
        let dy = toroidal_delta(cy, ty, height) as f32;
        let score = dx * hx + dy * hy;
        if score > best_score {
            best_score = score;
            best_tile = (tx, ty);
        }
    }

    Some(best_tile)
}

// ── Sense function ─────────────────────────────────────────────────

/// Map the decoded sense_radius gene (0.0-1.0) to 1-4 tiles.
fn mapped_sense_radius(genes: &DecodedGenes) -> u16 {
    let raw = (genes.get(genome::SENSE_RADIUS) * 3.0).ceil() as u16;
    raw.clamp(1, 4)
}

/// Scan the neighborhood around a cell and build a SenseResult.
///
/// Pure function — no RNG. Classification uses scaled thresholds.

// @veridikt
// purpose: "Scan a cell's neighborhood into a SenseResult: nearest food/threat/kin, counts, pheromone gradient, its own tile's share of the food around, and empty adjacent tiles for placement"
// triggers: Actions.sense_cached
// because: "What counts as 'food' depends on the cell's strongest acquisition gene (decay for scavengers, bright tiles for photosynthesizers, vent-proximity for thermosynthesizers), so each cell senses the resource it can actually use"
pub fn sense(
    cell: &Cell,
    genes: &DecodedGenes,
    world: &World,
    config: &WorldConfig,
) -> SenseResult {
    sense_cached(cell, genes, world, config, &mut DecodeCache::default())
}

/// `sense`, decoding neighbours through this tick's cache. Only
/// `flee_can_escape` decodes neighbours, to read their weapons.

// @veridikt
// purpose: "sense() with the tick's decode cache, which the Flee gate's threat list needs to read every non-kin neighbour's weapons"
// triggers: World.neighbors_in_radius, Genome.DecodeCache, Phase.apply_phase_modifiers
pub fn sense_cached(
    cell: &Cell,
    genes: &DecodedGenes,
    world: &World,
    config: &WorldConfig,
    cache: &mut DecodeCache,
) -> SenseResult {
    let (cx, cy) = cell.position;
    let radius = mapped_sense_radius(genes);

    let local_tile = TileSnapshot::from_tile(world.current_tile(cx, cy));

    // Kin recognition: precision scales the effective aggression trigger.
    // Low precision widens the "hostile" band.
    // Read straight from the genome, not the decoded genes: top-N gating
    // attenuates whatever is not in a cell's dozen strongest genes, and a
    // gated threshold of ~0.05 classified even siblings (distance ~0.13) as
    // threats. Recognition is a threshold, not an expressed capability.
    let precision = cell.genome.gene(genome::KIN_RECOGNITION_PRECISION) as f32 / 255.0;
    let aggression = cell.genome.gene(genome::AGGRESSION_TRIGGER) as f32 / 255.0;
    let effective_trigger = aggression * (0.5 + 0.5 * precision);

    // Determine which acquisition gene is strongest (for food detection).
    let photo = genes.get(genome::PHOTOSYNTHESIS_RATE);
    let thermo = genes.get(genome::THERMOSYNTHESIS_RATE);
    let scavenge = genes.get(genome::SCAVENGE_ABILITY);

    let neighbors = world.neighbors_in_radius(cx, cy, radius);

    let mut nearest_food: Option<(u16, u16)> = None;
    let mut nearest_food_dist = i32::MAX;
    let mut best_food_value = f32::NEG_INFINITY;
    // The most food on any one tile in range, for `own_food_share`.
    let mut richest_food = 0.0_f32;
    let mut nearest_threat: Option<(u16, u16, u32, u16)> = None;
    let mut nearest_threat_dist = i32::MAX;
    let mut nearest_kin: Option<(u16, u16, u32)> = None;
    let mut nearest_kin_dist = i32::MAX;
    let mut kin_count: u32 = 0;
    let mut threat_count: u32 = 0;
    let mut neighbor_count: u32 = 0;
    let mut threats: Vec<Threat> = Vec::new();
    let mut food_nearby = false;

    // Pheromone gradient tracking
    let mut best_pheromone = 0.0_f32;
    let mut pheromone_dir: (i32, i32) = (0, 0);

    for &(nx, ny, cell_id) in &neighbors {
        neighbor_count += 1;

        let neighbor_cell = world.get_cell(cell_id);
        let dist = genetic_distance(&cell.genome, &neighbor_cell.genome);
        let tile_dist = toroidal_dist(cx, cy, nx, ny, world.width, world.height);

        if dist < effective_trigger {
            kin_count += 1;
            if tile_dist < nearest_kin_dist {
                nearest_kin_dist = tile_dist;
                nearest_kin = Some((nx, ny, cell_id));
            }
        } else {
            threat_count += 1;
            if tile_dist < nearest_threat_dist {
                nearest_threat_dist = tile_dist;
                nearest_threat = Some((nx, ny, cell_id, tile_dist as u16));
            }
            if config.flee_can_escape {
                let mut t = cache.get(cell_id, &neighbor_cell.genome, config).clone();
                crate::sim::phase::apply_phase_modifiers(
                    &mut t,
                    &neighbor_cell.genome,
                    neighbor_cell.active_phase,
                );
                threats.push(Threat {
                    pos: (nx, ny),
                    cell_id,
                    dist: tile_dist as u16,
                    attack_power: t.get(genome::ATTACK_POWER),
                    venom: t.get(genome::VENOM),
                });
            }
        }
    }

    // Scan all tiles in radius for food and pheromone (including empty ones)
    let r = radius as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (wx, wy) = world.wrap(cx as i32 + dx, cy as i32 + dy);
            let tile = world.current_tile(wx, wy);

            // Track pheromone gradient
            if tile.pheromone > best_pheromone {
                best_pheromone = tile.pheromone;
                pheromone_dir = (dx, dy);
            }

            // Food detection based on strongest acquisition gene
            let is_food = if scavenge >= photo && scavenge >= thermo {
                tile.decay_energy > 0.0
            } else if photo >= thermo {
                tile.sunlight > 128
            } else {
                // Thermo: check vent proximity (bottom row, near vent x positions)
                is_near_vent(wx, wy, world, config.vent_radius)
            };

            if is_food {
                food_nearby = true;
                let tile_dist = toroidal_dist(cx, cy, wx, wy, world.width, world.height);
                // How much food is here, for `food_targets_richest`. Vents are
                // all-or-nothing, so for a thermosynthesizer every vent tile
                // scores the same and distance decides.
                let value = if scavenge >= photo && scavenge >= thermo {
                    tile.decay_energy
                } else if photo >= thermo {
                    tile.sunlight as f32
                } else {
                    1.0
                };
                richest_food = richest_food.max(value);
                let better = if config.food_targets_richest {
                    value > best_food_value
                        || (value == best_food_value && tile_dist < nearest_food_dist)
                } else {
                    tile_dist < nearest_food_dist
                };
                if better {
                    nearest_food_dist = tile_dist;
                    best_food_value = value;
                    nearest_food = Some((wx, wy));
                }
            }
        }
    }

    // Also check own tile for food
    let own_is_food = if scavenge >= photo && scavenge >= thermo {
        local_tile.decay_energy > 0.0
    } else if photo >= thermo {
        local_tile.sunlight > 128
    } else {
        is_near_vent(cx, cy, world, config.vent_radius)
    };
    if own_is_food {
        food_nearby = true;
    }
    // How much of the food around is under the cell itself: its own tile
    // against the richest other tile in range, on the same scale the food
    // scan uses. A predator's food is prey, which is never underfoot.
    let own_food = if !own_is_food {
        0.0
    } else if scavenge >= photo && scavenge >= thermo {
        local_tile.decay_energy
    } else if photo >= thermo {
        local_tile.sunlight as f32
    } else {
        1.0
    };
    let hunts = {
        let predation = genes.get(genome::PREDATION_EFFICIENCY);
        predation >= photo && predation >= thermo && predation >= scavenge
    };
    let own_food_share = if hunts || own_food <= 0.0 {
        0.0
    } else {
        own_food / (own_food + richest_food)
    };

    // Scale pheromone gradient by signal_sensitivity
    // A predator's food is prey, not a tile. Without this a hunter has no
    // food target at all and wanders while prey stands next to it.
    let predation = genes.get(genome::PREDATION_EFFICIENCY);
    if predation >= photo
        && predation >= thermo
        && predation >= scavenge
        && let Some((tx, ty, _, _)) = nearest_threat
    {
        nearest_food = Some((tx, ty));
        food_nearby = true;
    }

    let sensitivity = genes.get(genome::SIGNAL_SENSITIVITY);
    let pheromone_gradient = if best_pheromone > 0.0 && sensitivity > 0.0 {
        (
            (pheromone_dir.0 as f32 * sensitivity).round() as i8,
            (pheromone_dir.1 as f32 * sensitivity).round() as i8,
        )
    } else {
        (0, 0)
    };

    // Empty tiles for placement. `empty_adjacent` is radius 1, used by
    // movement and flee; `empty_scatter` reaches as far as the cell's
    // offspring_scatter gene allows, which is how a lineage disperses
    // instead of only ever budding into the tile next door.
    let scatter = mapped_offspring_scatter(genes);
    let mut empty_adjacent = Vec::new();
    let mut empty_scatter = Vec::new();
    for dy in -scatter..=scatter {
        for dx in -scatter..=scatter {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (wx, wy) = world.wrap(cx as i32 + dx, cy as i32 + dy);
            if world.current_tile(wx, wy).cell_id != 0 {
                continue;
            }
            empty_scatter.push((wx, wy));
            if dx.abs() <= 1 && dy.abs() <= 1 {
                empty_adjacent.push((wx, wy));
            }
        }
    }

    // How far each heading is clear, for moves longer than one tile. Only
    // worth walking when a move can actually go further than one tile.
    let mut free_run = [0u8; 8];
    let reach = (config.max_move_distance as i32).min(radius as i32).max(1);
    for (run, &(hx, hy)) in free_run.iter_mut().zip(HEADINGS.iter()) {
        for step in 1..=reach {
            let (wx, wy) = world.wrap(cx as i32 + hx * step, cy as i32 + hy * step);
            if world.current_tile(wx, wy).cell_id != 0 {
                break;
            }
            *run = step as u8;
        }
    }

    // What the nearest threat's armour would absorb, so `decide` can tell a
    // blow that lands from a harmless one.
    let nearest_threat_armor = match nearest_threat {
        Some((_, _, id, _)) if config.attack_only_when_harmful => {
            let target = world.get_cell(id);
            let mut t = target.genome.decode(config);
            crate::sim::phase::apply_phase_modifiers(&mut t, &target.genome, target.active_phase);
            t.get(genome::ARMOR) * 255.0
        }
        _ => 0.0,
    };

    SenseResult {
        nearest_food,
        nearest_threat,
        kin_count,
        threat_count,
        nearest_kin,
        pheromone_gradient,
        neighbor_count,
        food_nearby,
        local_tile,
        empty_adjacent,
        empty_scatter,
        free_run,
        nearest_threat_armor,
        threats,
        own_food_share,
        world_size: (world.width, world.height),
    }
}

/// Stretch a one-tile step from `from` to `to` along the same heading, as far
/// as `config.max_move_distance`, the mover's speed and the clear run allow.
///
/// `docs/spec.md` gene 6 keeps `speed` as the chance of moving at all; this is
/// how far the move goes once it happens: `max(1, round(speed * max))` tiles,
/// never through an occupied tile and never beyond the sense radius (a cell
/// cannot aim at what it cannot see — the same physical cap as
/// `attack_range <= sense_radius`).

// @veridikt
// purpose: "Extend a chosen one-tile move to a multi-tile move along the same heading, bounded by speed, max_move_distance, the clear run and the sense radius"
// because: "With one tile per tick as a hard ceiling, a forager could never cover the ground between intermittent food sources; spec.md keeps speed as a probability, so distance is a separate, configurable reach"
fn extend_move(
    from: (u16, u16),
    to: (u16, u16),
    genes: &DecodedGenes,
    sense: &SenseResult,
    config: &WorldConfig,
) -> (u16, u16) {
    if config.max_move_distance <= 1 {
        return to;
    }
    let (w, h) = sense.world_size;
    let dx = toroidal_delta(from.0, to.0, w).signum();
    let dy = toroidal_delta(from.1, to.1, h).signum();
    let Some(dir) = HEADINGS.iter().position(|&hd| hd == (dx, dy)) else {
        return to;
    };
    let wanted = (genes.get(genome::SPEED) * config.max_move_distance as f32).round() as i32;
    let steps = wanted.max(1).min(sense.free_run[dir] as i32).max(1);
    let x = (from.0 as i32 + dx * steps).rem_euclid(w as i32) as u16;
    let y = (from.1 as i32 + dy * steps).rem_euclid(h as i32) as u16;
    (x, y)
}

/// Check if a tile is near a thermal vent (within 1 tile of a vent on the bottom row).
fn is_near_vent(x: u16, y: u16, world: &World, radius: u32) -> bool {
    world.is_in_vent_zone(x, y, radius)
}

/// Chebyshev distance on a toroidal grid.
fn toroidal_dist(x1: u16, y1: u16, x2: u16, y2: u16, width: u32, height: u32) -> i32 {
    let dx = {
        let d = (x1 as i32 - x2 as i32).unsigned_abs() as i32;
        d.min(width as i32 - d)
    };
    let dy = {
        let d = (y1 as i32 - y2 as i32).unsigned_abs() as i32;
        d.min(height as i32 - d)
    };
    dx.max(dy)
}

/// Signed shortest-path delta from `from` to `to` on one toroidal axis.
///
/// At the seam the raw difference is `n - 1` where the real step is `-1`,
/// so headings and flee vectors must use this, never `to - from`.
fn toroidal_delta(from: u16, to: u16, n: u32) -> i32 {
    let n = n as i32;
    let d = (to as i32 - from as i32).rem_euclid(n);
    if d > n / 2 { d - n } else { d }
}

// ── Movement conflict resolution ──────────────────────────────────

/// A movement intent: a cell wants to move to a target tile.
#[derive(Debug, Clone, Copy)]
pub struct MoveIntent {
    /// Cell pool id of the moving cell.
    pub cell_id: u32,
    /// Target tile the cell wants to occupy.
    pub target: (u16, u16),
    /// Cell's rigidity gene value (0.0–1.0), used to break ties.
    pub rigidity: f32,
    /// Cell's original position (to stay in place if it loses).
    pub source: (u16, u16),
}

/// Result of resolving movement conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// Cell moves to target tile.
    Wins(u32, u16, u16),
    /// Cell stays at original position.
    Loses(u32, u16, u16),
}

/// Resolve movement conflicts: when multiple cells target the same tile,
/// highest rigidity wins. Returns a list of outcomes (winners and losers).
///
/// Ties in rigidity are broken by cell_id (lower id wins) for determinism.

// @veridikt
// purpose: "Arbitrate cells that want the same tile: highest rigidity wins it, everyone else stays put"
// because: "Ties break on lower cell_id, not iteration order, so the outcome is deterministic regardless of how the intents were collected"
pub fn resolve_movement_conflicts(intents: &[MoveIntent]) -> Vec<MoveOutcome> {
    use std::collections::HashMap;

    let mut groups: HashMap<(u16, u16), Vec<&MoveIntent>> = HashMap::new();

    for intent in intents {
        groups.entry(intent.target).or_default().push(intent);
    }

    let mut movement_outcomes: Vec<MoveOutcome> = Vec::new();

    for group in groups.values() {
        let mut winner = &group[0];
        for intent in &group[1..] {
            if intent.rigidity > winner.rigidity
                || intent.rigidity == winner.rigidity && intent.cell_id < winner.cell_id
            {
                winner = intent;
            }
        }
        for intent in group {
            if intent.cell_id == winner.cell_id {
                movement_outcomes.push(MoveOutcome::Wins(
                    intent.cell_id,
                    intent.target.0,
                    intent.target.1,
                ));
            } else {
                movement_outcomes.push(MoveOutcome::Loses(
                    intent.cell_id,
                    intent.source.0,
                    intent.source.1,
                ));
            }
        }
    }

    movement_outcomes
}

// ── Attack resolution ─────────────────────────────────────────────

/// Stats needed from each combatant for attack resolution.
#[derive(Debug, Clone, Copy)]
pub struct CombatStats {
    /// Cell pool id.
    pub cell_id: u32,
    /// Decoded `attack_power` gene (0.0–1.0).
    pub attack_power: f32,
    /// Decoded `armor` gene (0.0–1.0).
    pub armor: f32,
    /// Decoded `venom` gene (0.0–1.0). Only the attacker's venom is applied.
    pub venom: f32,
}

/// Result of a single attack encounter between two cells.
#[derive(Debug, Clone, Copy)]
pub struct AttackOutcome {
    /// Energy damage dealt to the attacker (from defender's retaliation).
    pub damage_to_attacker: f32,
    /// Energy damage dealt to the defender.
    pub damage_to_defender: f32,
    /// Venom ticks to apply to the defender (0 if attacker has no venom).
    pub venom_ticks: u8,
    /// Venom damage per tick applied to the defender.
    pub venom_damage: u8,
}

/// Resolve a single attack: simultaneous damage exchange + venom.
///
/// Damage formula: `attacker.attack_power * 255 - defender.armor * 255`, minimum 0.
/// Defender retaliates: `defender.attack_power * 255 - attacker.armor * 255`, minimum 0.
/// Venom: if `attacker.venom > 0`, defender gets `venom_ticks = (venom * 10) as u8`
/// and `venom_damage = (venom * 25) as u8`.

// @veridikt
// purpose: "Compute the outcome of one attack: simultaneous armor-reduced damage both ways, plus venom-over-time applied to the defender"
// because: "The defender always retaliates in the same exchange, so attacking an armored/strong cell can cost the aggressor more than it gains — aggression is not free"
pub fn resolve_attack(attacker: &CombatStats, defender: &CombatStats) -> AttackOutcome {
    AttackOutcome {
        damage_to_defender: (attacker.attack_power * 255.0 - defender.armor * 255.0).max(0.0),
        damage_to_attacker: (defender.attack_power * 255.0 - attacker.armor * 255.0).max(0.0),
        venom_ticks: (attacker.venom * 10.0) as u8,
        venom_damage: (attacker.venom * 25.0) as u8,
    }
}

// ── Reproduction resolution ───────────────────────────────────────

/// Result of a reproduction attempt.
#[derive(Debug)]
pub struct ReproductionOutcome {
    /// Child cell ready to be placed at the target tile.
    pub child: Cell,
    /// Energy remaining for the parent after splitting.
    pub parent_energy: f32,
    /// Cooldown ticks to set on the parent.
    pub parent_cooldown: u16,
}

/// Resolve a reproduction action: clone genome, mutate, split energy, set cooldown.
///
/// - Child genome = parent genome cloned + `mutate()` applied
/// - Child energy = `parent_energy * offspring_energy_share`, the share
///   bounded by `offspring_energy_share()` so neither side leaves dead
/// - Parent energy is reduced by child's energy
/// - Parent cooldown set from `reproduction_cooldown` gene
/// - Child is a fresh Cell at `target_pos` with age 0, no venom, no cooldown

// @veridikt
// purpose: "Produce a mutated child by cloning the parent genome, splitting off a share of the parent's energy, and setting the parent's cooldown"
// triggers: Genome.mutate, Actions.offspring_energy_share
// because: "Child energy is taken from the parent (offspring_energy_share, bounded by config), so reproduction is a real cost and over-reproducing can starve a lineage — births are paid for, not free"
pub fn resolve_reproduction(
    parent: &Cell,
    genes: &DecodedGenes,
    target_pos: (u16, u16),
    config: &WorldConfig,
    rng: &mut impl rand::Rng,
) -> ReproductionOutcome {
    let mut child_genome = parent.genome.clone();
    child_genome.mutate(config, rng);

    let child_energy = parent.energy * offspring_energy_share(genes, config);
    let child = Cell::new(child_genome, child_energy, target_pos);
    let parent_energy = parent.energy - child_energy;

    ReproductionOutcome {
        child,
        parent_energy,
        parent_cooldown: mapped_reproduction_cooldown(genes),
    }
}

// ── Share resolution ──────────────────────────────────────────────

/// Result of a share action.
#[derive(Debug, Clone, Copy)]
pub struct ShareOutcome {
    /// Energy remaining for the donor after sharing.
    pub donor_energy: f32,
    /// Energy for the recipient after receiving.
    pub recipient_energy: f32,
}

/// Resolve a share action: transfer energy from donor to recipient kin.
///
/// Transfer amount = `resource_sharing * donor_energy * 0.1`, capped so
/// donor doesn't go below zero.

// @veridikt
// purpose: "Move a fraction of the donor's energy to a kin recipient, scaled by the donor's resource_sharing gene"
// because: "Altruism is gated on kin recognition upstream, so sharing flows toward relatives — the mechanism that lets photosynthesizer/swarm colonies pool energy instead of competing"
pub fn resolve_share(donor: &Cell, donor_genes: &DecodedGenes, recipient: &Cell) -> ShareOutcome {
    let shared_energy = (donor_genes.get(genome::RESOURCE_SHARING) * donor.energy * 0.1).max(0.0);
    let donor_energy = donor.energy - shared_energy;
    let recipient_energy = recipient.energy + shared_energy;
    ShareOutcome {
        donor_energy,
        recipient_energy,
    }
}

// ── Placement helper ─────────────────────────────────────────────

/// Decoded genes with the cell's active-phase modifiers applied — what the
/// cell actually expresses this tick — served from this tick's decode cache.
///
/// `resolve_all` used plain `decode`, so the offense and defense phase
/// groups had no effect on anything: a cell could enter a "fight" phase and
/// still attack with its base stats.

// @veridikt
// purpose: "Produce the phase-modified decoded genes for a cell, so action resolution sees the same expression the cell decided with"
// depends_on: Genome.decode, Phase.apply_phase_modifiers, Genome.DecodeCache
fn effective_genes_cached(
    cell_id: u32,
    cell: &Cell,
    config: &WorldConfig,
    cache: &mut DecodeCache,
) -> DecodedGenes {
    let mut genes = cache.get(cell_id, &cell.genome, config).clone();
    crate::sim::phase::apply_phase_modifiers(&mut genes, &cell.genome, cell.active_phase);
    genes
}

/// Check whether a cell has already been placed in the next grid.
///
/// A cell is "placed" if the next-grid tile at its original position
/// already has its cell_id written.
fn is_placed(world: &World, cell_id: u32, pos: (u16, u16)) -> bool {
    world.next_tile(pos.0, pos.1).cell_id == cell_id
}

/// Copy a cell into the next grid at the given position, setting the
/// tile's cell_id and the cell's `position` so both stay in sync.

// @veridikt
// purpose: "Single write path for cell placement into next: sets the tile's cell_id and the cell's position field together"
// because: "Sensing, movement and every later placement read cell.position; if it lags the tile, the cell senses from a stale spot and gets re-placed there next tick (teleporting back or overwriting another cell)"
fn place_cell(world: &mut World, cell_id: u32, pos: (u16, u16)) {
    let occupant = world.next_tile(pos.0, pos.1).cell_id;
    debug_assert!(
        occupant == 0 || occupant == cell_id,
        "placing cell {cell_id} over cell {occupant} would leave a ghost"
    );
    let (width, height) = (world.width, world.height);
    let from = world.get_cell(cell_id).position;
    world.next_tile_mut(pos.0, pos.1).cell_id = cell_id;
    let cell = world.get_cell_mut(cell_id);
    cell.position = pos;

    // Remember which way the cell just went, so `compute_move_target` has
    // something for memory_length to weigh. `memory_dir` was never written,
    // which made gene 25 a no-op for every cell that ever lived.
    let (dx, dy) = (
        toroidal_delta(from.0, pos.0, width),
        toroidal_delta(from.1, pos.1, height),
    );
    if dx != 0 || dy != 0 {
        cell.memory_dir = (dx.signum() as i8, dy.signum() as i8);
        cell.memory_ticks = 0;
    }
}

/// Count down `memory_ticks` for one cell, clearing the remembered heading
/// when its `memory_length` has run out.
///
/// The gene is a *duration* (`docs/spec.md` 25, "ticks of directional
/// memory"), so the countdown is what distinguishes it from a plain weight.

// @veridikt
// purpose: "Age a cell's directional memory by one tick, clearing it once memory_length ticks have passed since the last move"
pub fn age_memory(cell: &mut Cell, genes: &DecodedGenes) {
    if cell.memory_dir == (0, 0) {
        return;
    }
    // memory_ticks counts *up* from the move that set the heading, so there
    // is no sentinel value to confuse with a real count.
    let span = (genes.get(genome::MEMORY_LENGTH) * u8::MAX as f32) as u8;
    if cell.memory_ticks >= span {
        cell.memory_dir = (0, 0);
        cell.memory_ticks = 0;
    } else {
        cell.memory_ticks += 1;
    }
}

// ── resolve_all orchestrator ─────────────────────────────────────

/// Process all actions simultaneously, writing results to the next grid.
///
/// Actions are resolved in priority order across ALL cells:
/// Phase 1: Reproduce — Phase 2: Attack — Phase 3: Flee/Move —
/// Phase 4: Share — Phase 5: Idle. With `flee_can_escape`, Flee/Move run
/// before Attack, and a blow whose target has moved out of reach misses.
///
/// Placement tracking: once a cell is placed in the next grid by any
/// phase, later phases skip it (no double-placement).

// @veridikt
// purpose: "Apply every cell's chosen action into the next grid in five global priority passes, with placement tracking so no cell is written twice"
// triggers: World.spawn_cell, World.deposit_decay, Genome.decode, Energy.corpse_decay_fade, Actions.resolve_moves, Actions.record_birth, Actions.record_attack, World.record_mut
// because: "Resolving by action-type across ALL cells (not cell-by-cell) is what makes the outcome independent of cell order; the `is_placed` guard prevents a later pass from overwriting a cell an earlier pass already committed"
pub fn resolve_all(
    actions: &[(u32, Action)],
    world: &mut World,
    config: &WorldConfig,
    rng: &mut impl rand::Rng,
    cache: &mut DecodeCache,
) {
    let tick = world.tick;

    // ── Phase 1: Reproduce ───────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Reproduce(tx, ty) = *action {
            // Target tile must still be empty in next grid
            if world.next_tile(tx, ty).cell_id != 0 {
                // Another reproduction already claimed this tile
                world.stats.repro_blocked += 1;
                place_cell(world, cell_id, world.get_cell(cell_id).position);
                continue;
            }

            let parent = world.get_cell(cell_id);
            let genes = effective_genes_cached(cell_id, parent, config, cache);
            let pos = parent.position;

            let outcome = resolve_reproduction(parent, &genes, (tx, ty), config, rng);

            // Place child in next grid
            let child_id = world.spawn_cell(outcome.child);
            cache.invalidate(child_id);
            place_cell(world, child_id, (tx, ty));

            // Update parent state and place at original position
            let parent_mut = world.get_cell_mut(cell_id);
            parent_mut.energy = outcome.parent_energy;
            parent_mut.cooldown_remaining = outcome.parent_cooldown;
            place_cell(world, cell_id, pos);
            record_birth(world, cell_id, child_id);
        }
    }

    // With `flee_can_escape`, cells move before any blow lands: every
    // action is simultaneous, and a strike aimed at where a cell stood misses
    // if it has left the attacker's reach. Otherwise the attack pass places
    // the defender first, and a cell under attack can never get away.
    if config.flee_can_escape {
        resolve_moves(actions, world, config, cache);
    }

    // ── Phase 2: Attack ──────────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Attack(target_id) = *action {
            let attacker_cell = world.get_cell(cell_id);
            let attacker_genes = effective_genes_cached(cell_id, attacker_cell, config, cache);
            let attacker_pos = attacker_cell.position;

            let defender_cell = world.get_cell(target_id);
            let defender_genes = effective_genes_cached(target_id, defender_cell, config, cache);
            let defender_pos = defender_cell.position;

            if config.flee_can_escape
                && toroidal_dist(
                    attacker_pos.0,
                    attacker_pos.1,
                    defender_pos.0,
                    defender_pos.1,
                    world.width,
                    world.height,
                ) > mapped_attack_range(&attacker_genes) as i32
            {
                world.stats.attacks_missed += 1;
                if !is_placed(world, cell_id, attacker_pos) {
                    place_cell(world, cell_id, attacker_pos);
                }
                continue;
            }

            let attacker_stats = CombatStats {
                cell_id,
                attack_power: attacker_genes.get(genome::ATTACK_POWER),
                armor: attacker_genes.get(genome::ARMOR),
                venom: attacker_genes.get(genome::VENOM),
            };
            let defender_stats = CombatStats {
                cell_id: target_id,
                attack_power: defender_genes.get(genome::ATTACK_POWER),
                armor: defender_genes.get(genome::ARMOR),
                venom: defender_genes.get(genome::VENOM),
            };

            let outcome = resolve_attack(&attacker_stats, &defender_stats);

            // Apply damage to attacker
            let attacker_energy_before = world.get_cell(cell_id).energy;
            let a = world.get_cell_mut(cell_id);
            a.energy -= outcome.damage_to_attacker;
            a.last_damage_tick = tick as u32;

            // Apply damage + venom to defender
            let defender_energy_before = world.get_cell(target_id).energy;
            let d = world.get_cell_mut(target_id);
            d.energy -= outcome.damage_to_defender;
            d.last_damage_tick = tick as u32;
            if outcome.venom_ticks > 0 {
                d.venom_ticks = outcome.venom_ticks;
                d.venom_damage = outcome.venom_damage;
            }
            let killed = !d.is_alive();

            // A kill feeds the killer: predation_efficiency of the victim's
            // energy is absorbed. Without this, attacking was pure loss for
            // both sides and no predator could ever pay for itself. The gene
            // scales within `max_predation_efficiency`, and with
            // `corpses_keep_energy` what the killer does not take stays in
            // the body for scavengers instead of vanishing.
            if killed && defender_energy_before > 0.0 {
                let share = attacker_genes.get(genome::PREDATION_EFFICIENCY)
                    * config.max_predation_efficiency;
                let absorbed = share * defender_energy_before;
                world.get_cell_mut(cell_id).energy += absorbed;
                world.stats.income[stats::PREDATION] += absorbed as f64;
                world.record_mut(cell_id).income[stats::PREDATION] += absorbed;
                let uneaten = defender_energy_before - absorbed;
                if config.corpses_keep_energy && uneaten > 0.0 {
                    let fade = crate::sim::energy::corpse_decay_fade(&defender_genes, config);
                    let idx = world.tile_index(defender_pos.0, defender_pos.1);
                    world.deposit_decay(idx, uneaten, fade);
                    world.stats.decay_deposited += uneaten as f64;
                }
            }
            // After the meal: an attacker hit back to zero can be revived by
            // what it absorbs, and is then not a combat death.
            record_attack(
                world,
                cell_id,
                target_id,
                &outcome,
                attacker_energy_before,
                defender_energy_before,
            );
            // ...and may absorb some of its genome with it (spec.md gene 44).
            if killed {
                let victim = world.get_cell(target_id).genome.clone();
                if world
                    .get_cell_mut(cell_id)
                    .genome
                    .absorb_from(&victim, config, rng)
                {
                    // The genome changed under the cache.
                    cache.invalidate(cell_id);
                }
            }

            // Place both at their original positions (if not already placed)
            if !is_placed(world, cell_id, attacker_pos) {
                place_cell(world, cell_id, attacker_pos);
            }
            if !is_placed(world, target_id, defender_pos) {
                place_cell(world, target_id, defender_pos);
            }
        }
    }

    // ── Phase 3: Flee + Move ─────────────────────────────────────
    if !config.flee_can_escape {
        resolve_moves(actions, world, config, cache);
    }

    // ── Phase 4: Share ───────────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Share(target_id) = *action {
            let donor = world.get_cell(cell_id);
            let donor_pos = donor.position;
            let recipient = world.get_cell(target_id);
            let recipient_pos = recipient.position;

            let donor_genes = effective_genes_cached(cell_id, donor, config, cache);
            let outcome = resolve_share(donor, &donor_genes, recipient);

            // Apply energy changes
            world.stats.shared += (world.get_cell(cell_id).energy - outcome.donor_energy) as f64;
            world.get_cell_mut(cell_id).energy = outcome.donor_energy;
            world.get_cell_mut(target_id).energy = outcome.recipient_energy;

            // Place both if not already placed
            if !is_placed(world, cell_id, donor_pos) {
                place_cell(world, cell_id, donor_pos);
            }
            if !is_placed(world, target_id, recipient_pos) {
                place_cell(world, target_id, recipient_pos);
            }
        }
    }

    // ── Phase 5: Idle ────────────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if *action == Action::Idle {
            let pos = world.get_cell(cell_id).position;
            if !is_placed(world, cell_id, pos) {
                place_cell(world, cell_id, pos);
            }
        }
    }
}

/// Resolve every Flee and Move into the next grid: conflicts over a tile go
/// to the higher rigidity, losers stay where they were. Cells already placed
/// by an earlier pass are left alone.

// @veridikt
// purpose: "Place every fleeing or moving cell that no earlier pass has placed, resolving contested target tiles by rigidity"
// triggers: Actions.resolve_movement_conflicts, Actions.place_cell
// because: "It runs after the attack pass by default, which pins an attacked cell where it stood; with flee_can_escape it runs before it, so a cell that moved out of reach is missed"
fn resolve_moves(
    actions: &[(u32, Action)],
    world: &mut World,
    config: &WorldConfig,
    cache: &mut DecodeCache,
) {
    let mut move_intents: Vec<MoveIntent> = Vec::new();

    for &(cell_id, ref action) in actions {
        let (tx, ty) = match *action {
            Action::Flee(x, y) | Action::Move(x, y) => (x, y),
            _ => continue,
        };

        // Skip cells already placed by earlier phases
        let cell = world.get_cell(cell_id);
        let pos = cell.position;
        if is_placed(world, cell_id, pos) {
            continue;
        }

        let genes = effective_genes_cached(cell_id, cell, config, cache);
        move_intents.push(MoveIntent {
            cell_id,
            target: (tx, ty),
            rigidity: genes.get(genome::RIGIDITY),
            source: pos,
        });
    }

    let move_outcomes = resolve_movement_conflicts(&move_intents);

    for outcome in &move_outcomes {
        match *outcome {
            // A newborn may already hold the target in next (targets are
            // only checked against current); the mover then stays put.
            MoveOutcome::Wins(cid, x, y) if world.next_tile(x, y).cell_id != 0 => {
                let source = world.get_cell(cid).position;
                place_cell(world, cid, source);
            }
            MoveOutcome::Wins(cid, x, y) => place_cell(world, cid, (x, y)),
            MoveOutcome::Loses(cid, x, y) => place_cell(world, cid, (x, y)),
        }
    }
}

/// Count a birth: the child joins its parent's lineage, and the tick stats
/// learn how far mutation moved it.

// @veridikt
// purpose: "Tag a newborn with its parent's lineage, count the birth and its mutation distance, and label a stillborn child or a parent that died giving birth"
// triggers: World.record_mut
// because: "Lineage inheritance is what lets the lab follow a founder's descendants; the labels exist because a zero-energy birth is a real death path the counters would otherwise miss"
fn record_birth(world: &mut World, parent_id: u32, child_id: u32) {
    let parent = world.get_cell(parent_id);
    let child = world.get_cell(child_id);
    let distance = genetic_distance(&parent.genome, &child.genome);
    let mutated = parent
        .genome
        .data
        .iter()
        .zip(child.genome.data.iter())
        .filter(|(a, b)| a != b)
        .count() as u32;
    let child_dead = !child.is_alive();
    let parent_dead = !parent.is_alive();
    let lineage = world.record(parent_id).lineage;
    world.record_mut(child_id).lineage = lineage;
    if child_dead {
        world.record_mut(child_id).death = Some(DeathCause::Stillborn);
    }
    if parent_dead {
        world.record_mut(parent_id).death = Some(DeathCause::Childbirth);
    }
    let s = &mut world.stats;
    s.births += 1;
    s.child_distance += distance as f64;
    s.child_mutated_bytes += mutated;
}

/// Count an attack, and label whoever it took from alive to dead. The death
/// itself is counted at cleanup, from the label: a cell struck to zero can
/// still be revived later in the tick, and the energy phase then clears the
/// label (`tick::record_energy`).

// @veridikt
// purpose: "Count an attack and its genetic distance, and label a defender or attacker the exchange took from alive to dead"
// triggers: World.record_mut
// because: "The death itself is counted at cleanup from the label, so a cell revived later in the tick is never counted as a kill"
fn record_attack(
    world: &mut World,
    attacker: u32,
    defender: u32,
    outcome: &AttackOutcome,
    attacker_energy_before: f32,
    defender_energy_before: f32,
) {
    let same_lineage = world.record(attacker).lineage == world.record(defender).lineage;
    let distance = genetic_distance(
        &world.get_cell(attacker).genome,
        &world.get_cell(defender).genome,
    );
    let defender_killed = defender_energy_before > 0.0 && !world.get_cell(defender).is_alive();
    let attacker_killed = attacker_energy_before > 0.0 && !world.get_cell(attacker).is_alive();
    let s = &mut world.stats;
    s.attacks += 1;
    s.same_lineage_attacks += same_lineage as u32;
    s.attack_distance += distance as f64;
    s.combat_damage += (outcome.damage_to_defender + outcome.damage_to_attacker) as f64;
    if defender_killed {
        world.record_mut(defender).death = Some(DeathCause::Combat);
    }
    if attacker_killed {
        world.record_mut(attacker).death = Some(DeathCause::Retaliation);
    }
}

// ── Tests ──────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;
    use crate::sim::genome::{GENOME_LEN, Genome};

    fn small_config() -> WorldConfig {
        WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 1,
            ..WorldConfig::default()
        }
    }

    fn make_genome(fill: u8) -> Genome {
        Genome::new([fill; GENOME_LEN])
    }

    fn make_cell_at(x: u16, y: u16, genome: Genome, energy: f32) -> Cell {
        Cell::new(genome, energy, (x, y))
    }

    // ── genetic_distance tests ─────────────────────────────────────

    #[test]
    fn genetic_distance_identical_is_zero() {
        let g = make_genome(100);
        assert!((genetic_distance(&g, &g)).abs() < f32::EPSILON);
    }

    #[test]
    fn genetic_distance_maximally_different_is_one() {
        let a = make_genome(0);
        let b = make_genome(255);
        assert!((genetic_distance(&a, &b) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn genetic_distance_is_symmetric() {
        let a = Genome::new({
            let mut d = [0u8; GENOME_LEN];
            d[0] = 100;
            d[5] = 200;
            d
        });
        let b = Genome::new({
            let mut d = [0u8; GENOME_LEN];
            d[0] = 50;
            d[5] = 150;
            d
        });
        assert!((genetic_distance(&a, &b) - genetic_distance(&b, &a)).abs() < f32::EPSILON);
    }

    #[test]
    fn genetic_distance_partial_difference() {
        let a = make_genome(100);
        let mut data_b = [100u8; GENOME_LEN];
        // Change only one gene by 50
        data_b[0] = 150;
        let b = Genome::new(data_b);
        let expected = 50.0 / (BASE_GENE_COUNT as f32 * 255.0);
        assert!((genetic_distance(&a, &b) - expected).abs() < 1e-6);
    }

    // ── TileSnapshot tests ─────────────────────────────────────────

    #[test]
    fn tile_snapshot_copies_fields() {
        let tile = Tile {
            cell_id: 5,
            decay_energy: 3.0,
            decay_fade: 0.02,
            pheromone: 1.5,
            toxin: 0.7,
            temperature: 200,
            sunlight: 180,
        };
        let snap = TileSnapshot::from_tile(&tile);
        assert_eq!(snap.sunlight, 180);
        assert_eq!(snap.temperature, 200);
        assert!((snap.decay_energy - 3.0).abs() < f32::EPSILON);
        assert!((snap.toxin - 0.7).abs() < f32::EPSILON);
        assert!((snap.pheromone - 1.5).abs() < f32::EPSILON);
    }

    // ── toroidal_dist tests ────────────────────────────────────────

    #[test]
    fn toroidal_dist_adjacent() {
        assert_eq!(toroidal_dist(5, 5, 6, 5, 16, 16), 1);
    }

    #[test]
    fn toroidal_dist_wraps() {
        // (0,0) to (15,0) on a 16-wide grid = distance 1 (wraps)
        assert_eq!(toroidal_dist(0, 0, 15, 0, 16, 16), 1);
    }

    #[test]
    fn toroidal_dist_diagonal() {
        // Chebyshev: max(dx, dy)
        assert_eq!(toroidal_dist(5, 5, 7, 8, 16, 16), 3);
    }

    // ── sense() tests ──────────────────────────────────────────────

    #[test]
    fn sense_detects_adjacent_cell() {
        let config = small_config();
        let mut world = World::new(&config);

        let genome_a = make_genome(100);
        let cell_a = make_cell_at(5, 5, genome_a.clone(), 50.0);

        // Place a neighbor at (6, 5) with a very different genome (threat)
        let genome_b = make_genome(0);
        let id_b = world.spawn_cell(make_cell_at(6, 5, genome_b, 50.0));
        world.set_current_tile_cell_id(6, 5, id_b);

        let genes = genome_a.decode(&config);
        let result = sense(&cell_a, &genes, &world, &config);

        assert_eq!(result.neighbor_count, 1);
    }

    #[test]
    fn sense_respects_radius() {
        let config = small_config();
        let mut world = World::new(&config);

        // Cell with minimum sense radius (gene = 0 -> radius 1)
        let mut data_a = [0u8; GENOME_LEN];
        data_a[genome::SENSE_RADIUS] = 0; // after decode pipeline -> low value -> radius 1
        let genome_a = Genome::new(data_a);
        let cell_a = make_cell_at(5, 5, genome_a.clone(), 50.0);

        // Place neighbor at distance 3 — outside radius 1
        let genome_b = make_genome(200);
        let id_b = world.spawn_cell(make_cell_at(8, 5, genome_b, 50.0));
        world.set_current_tile_cell_id(8, 5, id_b);

        let genes = genome_a.decode(&config);
        let result = sense(&cell_a, &genes, &world, &config);

        assert_eq!(
            result.neighbor_count, 0,
            "cell at distance 3 should be outside radius 1"
        );
    }

    #[test]
    fn sense_classifies_kin_vs_threat() {
        let config = small_config();
        let mut world = World::new(&config);

        // Cell with moderate aggression trigger
        let mut data_a = [128u8; GENOME_LEN];
        data_a[genome::AGGRESSION_TRIGGER] = 128;
        data_a[genome::KIN_RECOGNITION_PRECISION] = 255;
        let genome_a = Genome::new(data_a);
        let cell_a = make_cell_at(5, 5, genome_a.clone(), 50.0);

        // Similar neighbor (kin) — same fill value, small difference
        let mut data_kin = [128u8; GENOME_LEN];
        data_kin[0] = 130; // tiny difference
        let id_kin = world.spawn_cell(make_cell_at(6, 5, Genome::new(data_kin), 50.0));
        world.set_current_tile_cell_id(6, 5, id_kin);

        // Very different neighbor (threat)
        let genome_threat = make_genome(0);
        let id_threat = world.spawn_cell(make_cell_at(4, 5, genome_threat, 50.0));
        world.set_current_tile_cell_id(4, 5, id_threat);

        let genes = genome_a.decode(&config);
        let result = sense(&cell_a, &genes, &world, &config);

        assert_eq!(result.neighbor_count, 2);
        assert!(result.kin_count >= 1, "similar neighbor should be kin");
        assert!(
            result.threat_count >= 1,
            "different neighbor should be threat"
        );
    }

    #[test]
    fn sense_finds_empty_adjacent_tiles() {
        let config = small_config();
        let world = World::new(&config);

        let genome = make_genome(100);
        let cell = make_cell_at(5, 5, genome.clone(), 50.0);
        let genes = genome.decode(&config);

        let result = sense(&cell, &genes, &world, &config);

        // All 8 adjacent tiles should be empty in a fresh world
        assert_eq!(result.empty_adjacent.len(), 8);
    }

    #[test]
    fn sense_food_nearby_with_decay() {
        let config = small_config();
        let mut world = World::new(&config);

        // Cell specializing in scavenging
        let mut data = [0u8; GENOME_LEN];
        data[genome::SCAVENGE_ABILITY] = 255;
        data[genome::SENSE_RADIUS] = 255;
        let genome = Genome::new(data);
        let cell = make_cell_at(5, 5, genome.clone(), 50.0);

        // Place decay on adjacent tile
        let idx = world.tile_index(6, 5);
        world.current_grid_mut()[idx].decay_energy = 10.0;

        let genes = genome.decode(&config);
        let result = sense(&cell, &genes, &world, &config);

        assert!(
            result.food_nearby,
            "should detect decay as food for scavenger"
        );
        assert!(result.nearest_food.is_some());
    }

    #[test]
    fn sense_pheromone_gradient_direction() {
        let config = small_config();
        let mut world = World::new(&config);

        // Cell with signal sensitivity
        let mut data = [0u8; GENOME_LEN];
        data[genome::SIGNAL_SENSITIVITY] = 255;
        data[genome::SENSE_RADIUS] = 255;
        let genome = Genome::new(data);
        let cell = make_cell_at(5, 5, genome.clone(), 50.0);

        // Place high pheromone to the right
        let idx = world.tile_index(6, 5);
        world.current_grid_mut()[idx].pheromone = 10.0;

        let genes = genome.decode(&config);
        let result = sense(&cell, &genes, &world, &config);

        // Gradient should point right (positive x)
        assert!(
            result.pheromone_gradient.0 > 0,
            "gradient should point toward pheromone: {:?}",
            result.pheromone_gradient
        );
    }

    // ── decide() tests ─────────────────────────────────────────────

    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    /// Build a SenseResult with defaults for testing decide().
    fn base_sense_result() -> SenseResult {
        SenseResult {
            nearest_food: None,
            nearest_threat: None,
            kin_count: 0,
            threat_count: 0,
            nearest_kin: None,
            pheromone_gradient: (0, 0),
            neighbor_count: 0,
            food_nearby: false,
            local_tile: TileSnapshot {
                sunlight: 128,
                temperature: 128,
                decay_energy: 0.0,
                toxin: 0.0,
                pheromone: 0.0,
            },
            empty_adjacent: vec![(6, 5), (4, 5), (5, 6), (5, 4)],
            empty_scatter: vec![(6, 5), (4, 5), (5, 6), (5, 4)],
            free_run: [0; 8],
            nearest_threat_armor: 0.0,
            threats: Vec::new(),
            own_food_share: 0.0,
            world_size: (16, 16),
        }
    }

    #[test]
    fn decide_reproduce_when_above_threshold() {
        let config = small_config();
        // Cell with high energy, reproduction genes active
        let mut data = [0u8; GENOME_LEN];
        data[genome::REPRODUCTION_THRESHOLD] = 50;
        data[genome::ENERGY_STORAGE_CAP] = 200;
        data[genome::MATURITY_AGE] = 0; // always mature
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 200.0);
        cell.cooldown_remaining = 0;
        cell.age = 100;

        let genes = genome.decode(&config);
        let sense = base_sense_result();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert!(
            matches!(action, Action::Reproduce(_, _)),
            "should reproduce with high energy: got {:?}",
            action
        );
    }

    #[test]
    fn maturity_never_outlives_the_cell() {
        // A lineage that matures after it dies is sterile by construction:
        // seed 2 of the harness lost all 117 cells that way.
        let config = small_config();
        let mut data = [10u8; GENOME_LEN];
        data[genome::MATURITY_AGE] = 255; // matures as late as possible
        data[genome::MAX_AGE] = 0; // dies as early as possible
        let genes = Genome::new(data).decode(&config);

        let maturity = mapped_maturity_age(&genes, &config);
        let lifespan = crate::sim::energy::lifespan_ticks(&genes, &config);
        assert!(
            maturity < lifespan,
            "maturity {maturity} must fall inside lifespan {lifespan}"
        );
    }

    #[test]
    fn reproduction_threshold_respects_absolute_floor() {
        // A tiny-cap cell must not be able to split at near-zero energy.
        let config = small_config();
        let mut data = [10u8; GENOME_LEN];
        data[genome::ENERGY_STORAGE_CAP] = 0;
        data[genome::REPRODUCTION_THRESHOLD] = 0;
        let genes = Genome::new(data).decode(&config);

        let threshold = mapped_reproduction_threshold(&genes, &config);
        assert_eq!(threshold, config.reproduction_energy_floor);
    }

    #[test]
    fn decide_does_not_reproduce_on_cooldown() {
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::REPRODUCTION_THRESHOLD] = 50;
        data[genome::ENERGY_STORAGE_CAP] = 200;
        data[genome::MATURITY_AGE] = 0;
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 200.0);
        cell.cooldown_remaining = 10; // on cooldown
        cell.age = 100;

        let genes = genome.decode(&config);
        let sense = base_sense_result();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert!(
            !matches!(action, Action::Reproduce(_, _)),
            "should NOT reproduce while on cooldown: got {:?}",
            action
        );
    }

    #[test]
    fn decide_attack_when_threat_in_range() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::ATTACK_RANGE] = 128;
        data[genome::REPRODUCTION_THRESHOLD] = 255; // high threshold = won't reproduce
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 10.0);
        cell.cooldown_remaining = 1; // block reproduction gate

        let genes = genome.decode(&config);
        let mut sense = base_sense_result();
        sense.nearest_threat = Some((6, 5, 42, 1)); // adjacent threat

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert!(
            matches!(action, Action::Attack(42)),
            "should attack adjacent threat: got {:?}",
            action
        );
    }

    #[test]
    fn decide_flee_when_threat_out_of_attack_range() {
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::FLEE_RESPONSE] = 200;
        data[genome::ATTACK_RANGE] = 0; // minimal attack range
        data[genome::REPRODUCTION_THRESHOLD] = 255;
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 10.0);
        cell.cooldown_remaining = 1; // block reproduction gate

        let genes = genome.decode(&config);
        let mut sense = base_sense_result();
        sense.nearest_threat = Some((7, 5, 42, 2)); // threat at distance 2

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert!(
            matches!(action, Action::Flee(_, _)),
            "should flee from distant threat: got {:?}",
            action
        );
    }

    #[test]
    fn decide_move_with_speed() {
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 255; // always moves
        data[genome::REPRODUCTION_THRESHOLD] = 255;
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 10.0);
        cell.cooldown_remaining = 1; // block reproduction gate

        let genes = genome.decode(&config);
        let sense = base_sense_result();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert!(
            matches!(action, Action::Move(_, _)),
            "should move with high speed and no threats: got {:?}",
            action
        );
    }

    // ── resolve_movement_conflicts tests ──────────────────────────────

    #[test]
    fn chemotaxis_steers_toward_food() {
        // docs/spec.md gene 9: "tendency to move toward nearby energy
        // sources". The gene used to be spent on the pheromone gradient
        // while `nearest_food` was computed and never read, so nothing in
        // the simulation ever moved toward food.
        let config = small_config();
        let target_with = |chemotaxis: u8| {
            let mut data = [0u8; GENOME_LEN];
            data[genome::SPEED] = 255;
            data[genome::DIRECTION_BIAS] = 16; // heading east, tilted south
            data[genome::DIRECTION_NOISE] = 0;
            data[genome::CHEMOTAXIS_STRENGTH] = chemotaxis;
            let genome = Genome::new(data);
            let genes = genome.decode(&config);
            let cell = make_cell_at(5, 5, genome, 50.0);
            let mut sense = base_sense_result();
            sense.nearest_food = Some((5, 8)); // three tiles south
            let mut rng = ChaCha8Rng::seed_from_u64(3);
            compute_move_target(&cell, &genes, &sense, &mut rng).unwrap()
        };

        assert_eq!(
            target_with(0),
            (6, 5),
            "with no chemotaxis it follows its heading"
        );
        assert_eq!(
            target_with(255),
            (5, 6),
            "with chemotaxis it turns toward food"
        );
    }

    #[test]
    fn a_predator_hunts_the_nearest_prey() {
        // "predators roam aimlessly even with prey a few pixels away":
        // tile-based food detection never gave a hunter a target.
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::PREDATION_EFFICIENCY] = 255;
        data[genome::SENSE_RADIUS] = 255;
        data[genome::AGGRESSION_TRIGGER] = 0; // everything is a threat
        let genome = Genome::new(data);
        let genes = genome.decode(&config);

        let hunter = make_cell_at(5, 5, genome.clone(), 50.0);
        let prey = make_cell_at(7, 5, make_genome(200), 50.0);
        let (world, ids) = setup_world_with_cells(&config, vec![hunter, prey]);

        let sensed = sense(world.get_cell(ids[0]), &genes, &world, &config);
        assert_eq!(
            sensed.nearest_food,
            Some((7, 5)),
            "a predator's food is the nearest prey"
        );
    }

    #[test]
    fn direction_noise_is_symmetric() {
        // A one-sided noise term rotates every heading the same way, so a
        // cell heading +x picks the tile above far more often than the one
        // below. Symmetric noise splits them evenly.
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 255;
        data[genome::DIRECTION_BIAS] = 0; // heading 0 rad = +x
        data[genome::DIRECTION_NOISE] = 255;
        let genome = Genome::new(data);
        let genes = genome.decode(&config);
        let cell = make_cell_at(5, 5, genome, 50.0);
        let sense = base_sense_result();
        let mut rng = ChaCha8Rng::seed_from_u64(7);

        let (mut ccw, mut cw) = (0, 0);
        for _ in 0..4000 {
            match compute_move_target(&cell, &genes, &sense, &mut rng).unwrap() {
                (5, 6) => ccw += 1,
                (5, 4) => cw += 1,
                _ => {}
            }
        }
        let skew = (ccw - cw) as f32 / (ccw + cw) as f32;
        assert!(
            skew.abs() < 0.1,
            "turns should not favour one side: {ccw} vs {cw} (skew {skew})"
        );
    }

    #[test]
    fn move_target_uses_toroidal_delta_at_seam() {
        // A cell on the left edge heading -x must step across the seam to
        // x = width - 1, not be repelled by an unwrapped delta of +15.
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 255;
        data[genome::DIRECTION_BIAS] = 128; // ~pi rad = -x
        data[genome::DIRECTION_NOISE] = 0;
        let genome = Genome::new(data);
        let genes = genome.decode(&config);
        let cell = make_cell_at(0, 5, genome, 50.0);
        let mut sense = base_sense_result();
        sense.empty_adjacent = vec![(15, 5), (1, 5), (0, 4), (0, 6)];
        let mut rng = ChaCha8Rng::seed_from_u64(7);

        let target = compute_move_target(&cell, &genes, &sense, &mut rng).unwrap();
        assert_eq!(
            target,
            (15, 5),
            "should step across the seam, got {target:?}"
        );
    }

    #[test]
    fn flee_crosses_seam_away_from_threat() {
        // Threat at x = 15 is one tile to the cell's left across the seam,
        // so fleeing means stepping right, to x = 1.
        let cell = make_cell_at(0, 5, make_genome(128), 50.0);
        let mut sense = base_sense_result();
        sense.nearest_threat = Some((15, 5, 7, 1));
        sense.empty_adjacent = vec![(15, 4), (1, 5), (0, 6)];

        assert_eq!(flee_direction(&cell, (15, 5), &sense), Some((1, 5)));
    }

    #[test]
    fn toroidal_delta_takes_short_way() {
        assert_eq!(toroidal_delta(0, 15, 16), -1);
        assert_eq!(toroidal_delta(15, 0, 16), 1);
        assert_eq!(toroidal_delta(5, 6, 16), 1);
        assert_eq!(toroidal_delta(5, 5, 16), 0);
    }

    #[test]
    fn movement_no_conflict_all_win() {
        let intents = vec![
            MoveIntent {
                cell_id: 1,
                target: (3, 3),
                rigidity: 0.5,
                source: (2, 3),
            },
            MoveIntent {
                cell_id: 2,
                target: (5, 5),
                rigidity: 0.5,
                source: (4, 5),
            },
        ];
        let outcomes = resolve_movement_conflicts(&intents);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.contains(&MoveOutcome::Wins(1, 3, 3)));
        assert!(outcomes.contains(&MoveOutcome::Wins(2, 5, 5)));
    }

    #[test]
    fn movement_conflict_higher_rigidity_wins() {
        let intents = vec![
            MoveIntent {
                cell_id: 1,
                target: (5, 5),
                rigidity: 0.3,
                source: (4, 5),
            },
            MoveIntent {
                cell_id: 2,
                target: (5, 5),
                rigidity: 0.8,
                source: (6, 5),
            },
        ];
        let outcomes = resolve_movement_conflicts(&intents);
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes.contains(&MoveOutcome::Wins(2, 5, 5)),
            "higher rigidity should win"
        );
        assert!(
            outcomes.contains(&MoveOutcome::Loses(1, 4, 5)),
            "lower rigidity should lose (stay at source)"
        );
    }

    #[test]
    fn movement_conflict_tie_broken_by_cell_id() {
        let intents = vec![
            MoveIntent {
                cell_id: 10,
                target: (5, 5),
                rigidity: 0.5,
                source: (4, 5),
            },
            MoveIntent {
                cell_id: 3,
                target: (5, 5),
                rigidity: 0.5,
                source: (6, 5),
            },
        ];
        let outcomes = resolve_movement_conflicts(&intents);
        assert!(
            outcomes.contains(&MoveOutcome::Wins(3, 5, 5)),
            "lower cell_id should win ties"
        );
        assert!(outcomes.contains(&MoveOutcome::Loses(10, 4, 5)));
    }

    #[test]
    fn movement_three_way_conflict() {
        let intents = vec![
            MoveIntent {
                cell_id: 1,
                target: (5, 5),
                rigidity: 0.2,
                source: (4, 5),
            },
            MoveIntent {
                cell_id: 2,
                target: (5, 5),
                rigidity: 0.9,
                source: (6, 5),
            },
            MoveIntent {
                cell_id: 3,
                target: (5, 5),
                rigidity: 0.5,
                source: (5, 4),
            },
        ];
        let outcomes = resolve_movement_conflicts(&intents);
        assert_eq!(outcomes.len(), 3);
        assert!(
            outcomes.contains(&MoveOutcome::Wins(2, 5, 5)),
            "highest rigidity wins"
        );
        assert!(outcomes.contains(&MoveOutcome::Loses(1, 4, 5)));
        assert!(outcomes.contains(&MoveOutcome::Loses(3, 5, 4)));
    }

    #[test]
    fn movement_empty_intents() {
        let outcomes = resolve_movement_conflicts(&[]);
        assert!(outcomes.is_empty());
    }

    #[test]
    fn decide_idle_when_no_speed() {
        let config = small_config();
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 0; // sessile
        data[genome::REPRODUCTION_THRESHOLD] = 255;
        let genome = Genome::new(data);
        let mut cell = make_cell_at(5, 5, genome.clone(), 10.0);
        cell.cooldown_remaining = 1; // block reproduction gate

        let genes = genome.decode(&config);
        let sense = base_sense_result();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let action = decide(&cell, &genes, &sense, &config, &mut rng);
        assert_eq!(action, Action::Idle, "sessile cell should idle");
    }

    // ── resolve_attack tests ──────────────────────────────────────────

    fn make_combat_stats(cell_id: u32, attack: f32, armor: f32, venom: f32) -> CombatStats {
        CombatStats {
            cell_id,
            attack_power: attack,
            armor,
            venom,
        }
    }

    #[test]
    fn attack_both_take_damage() {
        let attacker = make_combat_stats(1, 0.8, 0.2, 0.0);
        let defender = make_combat_stats(2, 0.5, 0.3, 0.0);
        let outcome = resolve_attack(&attacker, &defender);

        // Attacker deals: 0.8*255 - 0.3*255 = 127.5
        let expected_to_defender = (0.8 - 0.3) * 255.0;
        assert!((outcome.damage_to_defender - expected_to_defender).abs() < 1e-3);

        // Defender retaliates: 0.5*255 - 0.2*255 = 76.5
        let expected_to_attacker = (0.5 - 0.2) * 255.0;
        assert!((outcome.damage_to_attacker - expected_to_attacker).abs() < 1e-3);
    }

    #[test]
    fn attack_armor_negates_damage() {
        // Defender has higher armor than attacker's power
        let attacker = make_combat_stats(1, 0.3, 0.0, 0.0);
        let defender = make_combat_stats(2, 0.0, 0.8, 0.0);
        let outcome = resolve_attack(&attacker, &defender);

        assert!(
            outcome.damage_to_defender < f32::EPSILON,
            "armor >= attack should mean zero damage, got {}",
            outcome.damage_to_defender
        );
    }

    #[test]
    fn attack_venom_applied_to_defender() {
        let attacker = make_combat_stats(1, 0.5, 0.5, 0.6);
        let defender = make_combat_stats(2, 0.5, 0.5, 0.0);
        let outcome = resolve_attack(&attacker, &defender);

        assert_eq!(outcome.venom_ticks, (0.6 * 10.0) as u8);
        assert_eq!(outcome.venom_damage, (0.6 * 25.0) as u8);
    }

    #[test]
    fn attack_no_venom_when_zero() {
        let attacker = make_combat_stats(1, 0.5, 0.5, 0.0);
        let defender = make_combat_stats(2, 0.5, 0.5, 0.0);
        let outcome = resolve_attack(&attacker, &defender);

        assert_eq!(outcome.venom_ticks, 0);
        assert_eq!(outcome.venom_damage, 0);
    }

    #[test]
    fn attack_defender_venom_not_applied() {
        // Only attacker's venom matters, not defender's
        let attacker = make_combat_stats(1, 0.5, 0.5, 0.0);
        let defender = make_combat_stats(2, 0.5, 0.5, 0.9);
        let outcome = resolve_attack(&attacker, &defender);

        assert_eq!(
            outcome.venom_ticks, 0,
            "defender's venom should not affect attacker"
        );
    }

    // ── resolve_reproduction tests ────────────────────────────────────

    #[test]
    fn reproduction_energy_split() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::OFFSPRING_ENERGY_SHARE] = 128; // ~0.5 after decode
        data[genome::REPRODUCTION_COOLDOWN] = 128;
        let genome = Genome::new(data);
        let parent = make_cell_at(5, 5, genome.clone(), 100.0);
        let genes = genome.decode(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &config, &mut rng);

        let share = genes.get(genome::OFFSPRING_ENERGY_SHARE);
        let expected_child = 100.0 * share;
        assert!(
            (outcome.child.energy - expected_child).abs() < 1e-3,
            "child energy should be parent * share: got {}",
            outcome.child.energy
        );
        assert!(
            (outcome.parent_energy - (100.0 - expected_child)).abs() < 1e-3,
            "parent energy should be reduced by child's: got {}",
            outcome.parent_energy
        );
    }

    /// A share that decodes to 0 made a zero-energy child: counted as a
    /// birth, dead at cleanup, and leaving `corpse_biomass` of decay made from
    /// nothing. A share of 1 left the parent at zero, dead in childbirth. The
    /// share is now bounded, so both are born alive — and alive with enough
    /// to survive the tick they are born in, not just above zero.
    #[test]
    fn neither_child_nor_parent_is_born_dead_whatever_the_share_gene() {
        let config = small_config();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let parent_energy = config.reproduction_energy_floor;
        for byte in [0u8, 255] {
            let mut data = [0u8; GENOME_LEN];
            data[genome::OFFSPRING_ENERGY_SHARE] = byte;
            let genome = Genome::new(data);
            let genes = genome.decode(&config);
            // The genome really does decode to the degenerate end.
            assert_eq!(
                genes.get(genome::OFFSPRING_ENERGY_SHARE),
                byte as f32 / 255.0
            );
            let parent = make_cell_at(5, 5, genome, parent_energy);

            let outcome = resolve_reproduction(&parent, &genes, (6, 5), &config, &mut rng);

            let floor = parent_energy * config.min_offspring_energy_share;
            let kept = parent_energy * (1.0 - config.max_offspring_energy_share);
            assert!(
                outcome.child.energy >= floor - 1e-4,
                "share byte {byte}: child born with {} energy (floor {floor})",
                outcome.child.energy
            );
            assert!(
                outcome.parent_energy >= kept - 1e-4,
                "share byte {byte}: parent left with {} energy (floor {kept})",
                outcome.parent_energy
            );
            // What a cell of this genome burns in a tick: both must outlive
            // the energy phase of the tick they split in.
            let drain = crate::sim::energy::metabolic_cost(&genes, 128, &config);
            assert!(outcome.child.energy > drain && outcome.parent_energy > drain);
        }
    }

    #[test]
    fn reproduction_child_placed_at_target() {
        let config = small_config();
        let data = [128u8; GENOME_LEN];
        let genome = Genome::new(data);
        let parent = make_cell_at(5, 5, genome.clone(), 100.0);
        let genes = genome.decode(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &config, &mut rng);

        assert_eq!(outcome.child.position, (6, 5));
    }

    #[test]
    fn reproduction_cooldown_set() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::REPRODUCTION_COOLDOWN] = 200;
        let genome = Genome::new(data);
        let parent = make_cell_at(5, 5, genome.clone(), 100.0);
        let genes = genome.decode(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &config, &mut rng);
        let expected_cooldown = mapped_reproduction_cooldown(&genes);

        assert_eq!(outcome.parent_cooldown, expected_cooldown);
        assert!(outcome.parent_cooldown > 0, "cooldown should be non-zero");
    }

    #[test]
    fn reproduction_child_is_fresh() {
        let config = small_config();
        let data = [128u8; GENOME_LEN];
        let genome = Genome::new(data);
        let mut parent = make_cell_at(5, 5, genome.clone(), 100.0);
        parent.age = 500;
        parent.venom_ticks = 3;
        let genes = genome.decode(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &config, &mut rng);

        assert_eq!(outcome.child.age, 0, "child should start at age 0");
        assert_eq!(outcome.child.venom_ticks, 0, "child should have no venom");
        assert_eq!(outcome.child.cooldown_remaining, 0, "child has no cooldown");
    }

    // ── resolve_share tests ──────────────────────────────────────────

    #[test]
    fn share_transfers_energy() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::RESOURCE_SHARING] = 200; // high sharing
        let genome = Genome::new(data);
        let donor = make_cell_at(5, 5, genome.clone(), 100.0);
        let recipient = make_cell_at(6, 5, make_genome(128), 20.0);
        let genes = genome.decode(&config);

        let outcome = resolve_share(&donor, &genes, &recipient);
        let sharing = genes.get(genome::RESOURCE_SHARING);
        let expected_transfer = sharing * 100.0 * 0.1;

        assert!(
            (outcome.donor_energy - (100.0 - expected_transfer)).abs() < 1e-3,
            "donor should lose transfer amount: got {}",
            outcome.donor_energy
        );
        assert!(
            (outcome.recipient_energy - (20.0 + expected_transfer)).abs() < 1e-3,
            "recipient should gain transfer amount: got {}",
            outcome.recipient_energy
        );
    }

    #[test]
    fn share_zero_sharing_gene_transfers_nothing() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::RESOURCE_SHARING] = 0;
        let genome = Genome::new(data);
        let donor = make_cell_at(5, 5, genome.clone(), 100.0);
        let recipient = make_cell_at(6, 5, make_genome(128), 20.0);
        let genes = genome.decode(&config);

        let outcome = resolve_share(&donor, &genes, &recipient);

        assert!((outcome.donor_energy - 100.0).abs() < 1e-3);
        assert!((outcome.recipient_energy - 20.0).abs() < 1e-3);
    }

    // ── resolve_all tests ────────────────────────────────────────────

    /// Helper: set up a world with cells placed on the current grid.
    /// Returns (world, cell_ids).
    fn setup_world_with_cells(config: &WorldConfig, cells: Vec<Cell>) -> (World, Vec<u32>) {
        let mut world = World::new(config);
        let mut ids = Vec::new();
        for cell in cells {
            let pos = cell.position;
            let id = world.spawn_cell(cell);
            world.set_current_tile_cell_id(pos.0, pos.1, id);
            ids.push(id);
        }
        (world, ids)
    }

    #[test]
    fn resolve_all_idle_places_cell() {
        let config = small_config();
        let cell = make_cell_at(5, 5, make_genome(128), 50.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![cell]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let actions = vec![(ids[0], Action::Idle)];
        resolve_all(
            &actions,
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        assert_eq!(
            world.next_tile(5, 5).cell_id,
            ids[0],
            "idle cell should be placed at original position in next grid"
        );
    }

    #[test]
    fn resolve_all_move_places_at_target() {
        let config = small_config();
        let cell = make_cell_at(5, 5, make_genome(128), 50.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![cell]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let actions = vec![(ids[0], Action::Move(6, 5))];
        resolve_all(
            &actions,
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        assert_eq!(
            world.next_tile(6, 5).cell_id,
            ids[0],
            "moving cell should appear at target in next grid"
        );
        assert_eq!(
            world.next_tile(5, 5).cell_id,
            0,
            "original position should be empty in next grid"
        );
    }

    #[test]
    fn a_move_writes_the_remembered_heading_and_it_expires() {
        // spec.md gene 25: "ticks of directional memory. 0 = purely
        // reactive." `memory_dir` was read by compute_move_target but never
        // written, so the term was always zero for every cell.
        let config = WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 0,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);
        let mut data = [0u8; GENOME_LEN];
        data[genome::MEMORY_LENGTH] = 255;
        let id = world.spawn_cell(Cell::new(Genome::new(data), 50.0, (5, 5)));
        world.set_current_tile_cell_id(5, 5, id);
        world.prepare_next();

        place_cell(&mut world, id, (6, 5));
        assert_eq!(world.get_cell(id).memory_dir, (1, 0));

        // A long memory_length keeps it for many ticks; a zero one drops it
        // on the first.
        let long = Genome::new(data).decode(&config);
        data[genome::MEMORY_LENGTH] = 0;
        let short = Genome::new(data).decode(&config);

        let mut forgetful = world.get_cell(id).clone();
        age_memory(&mut forgetful, &short);
        assert_eq!(
            forgetful.memory_dir,
            (0, 0),
            "a memory_length-0 cell kept a heading"
        );

        let mut persistent = world.get_cell(id).clone();
        for _ in 0..20 {
            age_memory(&mut persistent, &long);
        }
        assert_eq!(
            persistent.memory_dir,
            (1, 0),
            "a long memory expired in 20 ticks"
        );
    }

    #[test]
    fn adhesion_holds_a_cell_among_its_kin() {
        // spec.md gene 28: "tendency to stick to adjacent genetically
        // similar cells. Enables cluster formation." Never read, so nothing
        // held a colony together.
        let config = WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 0,
            ..WorldConfig::default()
        };
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 255;
        let free = Genome::new(data).decode(&config);
        data[genome::ADHESION] = 255;
        let sticky = Genome::new(data).decode(&config);

        let cell = Cell::new(Genome::new(data), 10.0, (5, 5));
        let mut sense = base_sense_result();
        sense.neighbor_count = 8;
        sense.kin_count = 8; // entirely surrounded by relatives

        let moves = |genes: &DecodedGenes, seed: u64| {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            (0..400)
                .filter(|_| {
                    matches!(
                        decide(&cell, genes, &sense, &config, &mut rng),
                        Action::Move(..)
                    )
                })
                .count()
        };
        let loose = moves(&free, 4);
        let stuck = moves(&sticky, 4);
        assert!(
            stuck < loose / 2,
            "a fully adhesive cell moved {stuck} times of 400 against {loose} for a free one"
        );
    }

    #[test]
    fn a_fast_cell_covers_several_tiles_but_never_through_another_cell() {
        // speed stays the chance of moving (spec.md gene 6); max_move_distance
        // sets how far a move goes. With it at 1 nothing changes.
        let mut data = [0u8; GENOME_LEN];
        data[genome::SPEED] = 255;
        let config = WorldConfig {
            max_move_distance: 4,
            ..WorldConfig::default()
        };
        let genes = Genome::new(data).decode(&config);
        let east = HEADINGS.iter().position(|&h| h == (1, 0)).unwrap();

        let mut sense = base_sense_result();
        sense.free_run[east] = 4;
        assert_eq!(extend_move((5, 5), (6, 5), &genes, &sense, &config), (9, 5));

        // Blocked after two tiles: stop in front of the obstacle.
        sense.free_run[east] = 2;
        assert_eq!(extend_move((5, 5), (6, 5), &genes, &sense, &config), (7, 5));

        // The spec's one-tile world is untouched.
        let one = WorldConfig::default();
        sense.free_run[east] = 4;
        assert_eq!(extend_move((5, 5), (6, 5), &genes, &sense, &one), (6, 5));
    }

    #[test]
    fn harmless_prey_runs_from_an_adjacent_threat_instead_of_fighting_it() {
        // Attack is gated before Flee, so a cell with a threat adjacent
        // "attacked" it however harmless its blow — prey fought predators
        // instead of running. With attack_only_when_harmful it flees.
        let mut data = [0u8; GENOME_LEN];
        data[genome::FLEE_RESPONSE] = 255;
        let cell = Cell::new(Genome::new(data), 10.0, (5, 5));
        let mut sense = base_sense_result();
        sense.nearest_threat = Some((6, 5, 42, 1));
        sense.nearest_threat_armor = 50.0;

        let old = WorldConfig::default();
        let genes = Genome::new(data).decode(&old);
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        assert!(matches!(
            decide(&cell, &genes, &sense, &old, &mut rng),
            Action::Attack(42)
        ));

        let new = WorldConfig {
            attack_only_when_harmful: true,
            ..WorldConfig::default()
        };
        assert!(
            matches!(
                decide(&cell, &genes, &sense, &new, &mut rng),
                Action::Flee(..)
            ),
            "a cell that cannot get through the threat's armour still attacked it"
        );
    }

    /// Flee had no roll and no judgement: any non-kin in sight sent a cell
    /// with any flee_response at all running, every tick — including from a
    /// neighbour whose blow its armour stops. With `flee_can_escape` a cell
    /// runs only from what can hurt it (a blow through its armour, or venom),
    /// and only as often as `speed * flee_response`.
    #[test]
    fn a_cell_flees_only_from_what_can_hurt_it_and_only_as_often_as_it_can_move() {
        let config = WorldConfig {
            flee_can_escape: true,
            ..small_config()
        };
        let flights = |data: [u8; GENOME_LEN], attack: f32, venom: f32| {
            let genome = Genome::new(data);
            let genes = genome.decode(&config);
            let mut cell = make_cell_at(5, 5, genome, 10.0);
            cell.cooldown_remaining = 1; // no reproduction
            // Two tiles away: out of the cell's own attack range (1), so the
            // Attack gate never takes it first.
            let mut sense = base_sense_result();
            sense.nearest_threat = Some((7, 5, 42, 2));
            sense.threats = vec![Threat {
                pos: (7, 5),
                cell_id: 42,
                dist: 2,
                attack_power: attack,
                venom,
            }];
            let mut rng = ChaCha8Rng::seed_from_u64(7);
            let n = (0..200)
                .filter(|_| {
                    matches!(
                        decide(&cell, &genes, &sense, &config, &mut rng),
                        Action::Flee(..)
                    )
                })
                .count();
            (n, genes)
        };

        let mut runner = [0u8; GENOME_LEN];
        runner[genome::FLEE_RESPONSE] = 255;
        runner[genome::SPEED] = 255;
        runner[genome::ARMOR] = 100;
        let armor = Genome::new(runner).decode(&config).get(genome::ARMOR);
        assert!(armor > 0.05, "the runner needs some armour to test against");

        let (harmless, _) = flights(runner, armor * 0.5, 0.0);
        assert_eq!(harmless, 0, "ran from a blow its armour stops");

        let (strong, genes) = flights(runner, armor + 0.2, 0.0);
        let p = genes.get(genome::SPEED) * genes.get(genome::FLEE_RESPONSE);
        assert!(p < 0.9, "the test needs a roll that can fail, p = {p}");
        let expected = p * 200.0;
        assert!(
            (strong as f32 - expected).abs() < 30.0,
            "fled {strong}/200 times from a real threat, expected about {expected:.0}"
        );

        let (venomous, _) = flights(runner, 0.0, 0.5);
        assert!(
            venomous > 0,
            "venom gets through any armour, so it is worth running from"
        );

        let mut sessile = runner;
        sessile[genome::SPEED] = 0;
        let (stuck, _) = flights(sessile, armor + 0.2, 0.0);
        assert_eq!(stuck, 0, "a cell with no speed cannot flee");
    }

    /// The attack pass placed the defender before Flee/Move ran, and those
    /// skip placed cells, so an attacked cell could never get away. With
    /// `flee_can_escape` movement resolves first: a blow at a cell that has
    /// left the attacker's reach misses, and one still in reach lands.
    #[test]
    fn a_cell_that_flees_out_of_reach_is_missed() {
        let run = |escape: bool, flee_to: (u16, u16)| {
            let config = WorldConfig {
                flee_can_escape: escape,
                ..small_config()
            };
            let mut att = [0u8; GENOME_LEN];
            att[genome::ATTACK_POWER] = 255; // attack_range 0: reach 1
            let attacker = make_cell_at(5, 5, Genome::new(att), 50.0);
            let prey = make_cell_at(6, 5, make_genome(0), 40.0);
            let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, prey]);
            let mut rng = ChaCha8Rng::seed_from_u64(3);
            resolve_all(
                &[
                    (ids[0], Action::Attack(ids[1])),
                    (ids[1], Action::Flee(flee_to.0, flee_to.1)),
                ],
                &mut world,
                &config,
                &mut rng,
                &mut DecodeCache::default(),
            );
            let prey = world.get_cell(ids[1]);
            (prey.position, prey.energy, world.stats.attacks_missed)
        };

        let (pos, energy, _) = run(false, (7, 5));
        assert_eq!(pos, (6, 5), "without the switch the prey is pinned");
        assert!(energy <= 0.0, "...and killed where it stood");

        let (pos, energy, missed) = run(true, (7, 5));
        assert_eq!(pos, (7, 5), "the prey should have got away");
        assert_eq!(energy, 40.0, "a blow at an empty tile hurt the prey");
        assert_eq!(missed, 1);

        // A flight that stays inside the attacker's reach does not save it.
        let (pos, energy, missed) = run(true, (6, 4));
        assert_eq!(pos, (6, 4));
        assert!(energy <= 0.0, "a prey still in reach was not hit");
        assert_eq!(missed, 0);
    }

    #[test]
    fn a_forager_can_steer_to_the_richest_food_rather_than_the_nearest_trace() {
        let base = WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 0,
            initial_decay_matter: 0.0,
            ..WorldConfig::default()
        };
        let mut world = World::new(&base);
        let mut data = [0u8; GENOME_LEN];
        data[genome::SCAVENGE_ABILITY] = 255;
        data[genome::SENSE_RADIUS] = 255;
        let id = world.spawn_cell(Cell::new(Genome::new(data), 50.0, (5, 5)));
        world.set_current_tile_cell_id(5, 5, id);
        let near = world.tile_index(6, 5);
        let far = world.tile_index(8, 5);
        world.current_grid_mut()[near].decay_energy = 0.5;
        world.current_grid_mut()[far].decay_energy = 25.0;

        let genes = Genome::new(data).decode(&base);
        let nearest = sense(world.get_cell(id), &genes, &world, &base);
        assert_eq!(nearest.nearest_food, Some((6, 5)));

        let richest_cfg = WorldConfig {
            food_targets_richest: true,
            ..base.clone()
        };
        let richest = sense(world.get_cell(id), &genes, &world, &richest_cfg);
        assert_eq!(
            richest.nearest_food,
            Some((8, 5)),
            "steered to the trace, not the corpse"
        );
    }

    /// The Move gate rolled against speed whatever a cell stood on, and the
    /// food scan skips the cell's own tile, so chemotaxis even steered a
    /// mobile scavenger off a fresh corpse toward the nearest trace. With
    /// `foragers_stay_on_food` its own chemotaxis holds it on the meal, and
    /// lets it go once the tile is eaten out.
    #[test]
    fn a_forager_stays_on_a_meal_and_leaves_an_empty_tile() {
        let base = WorldConfig {
            grid_width: 16,
            grid_height: 16,
            vent_count: 0,
            initial_decay_matter: 0.0,
            ..WorldConfig::default()
        };
        let mut data = [0u8; GENOME_LEN];
        data[genome::SCAVENGE_ABILITY] = 255;
        data[genome::SPEED] = 255;
        data[genome::CHEMOTAXIS_STRENGTH] = 255;
        data[genome::SENSE_RADIUS] = 128;
        let moves = |config: &WorldConfig, underfoot: f32| {
            let mut world = World::new(config);
            let mut cell = Cell::new(Genome::new(data), 30.0, (5, 5));
            cell.cooldown_remaining = 1; // no reproduction
            let id = world.spawn_cell(cell);
            world.set_current_tile_cell_id(5, 5, id);
            let (here, trace) = (world.tile_index(5, 5), world.tile_index(7, 5));
            world.current_grid_mut()[here].decay_energy = underfoot;
            world.current_grid_mut()[trace].decay_energy = 2.0;
            let genes = Genome::new(data).decode(config);
            let sensed = sense(world.get_cell(id), &genes, &world, config);
            let mut rng = ChaCha8Rng::seed_from_u64(9);
            let cell = world.get_cell(id);
            let n = (0..200)
                .filter(|_| {
                    matches!(
                        decide(cell, &genes, &sensed, config, &mut rng),
                        Action::Move(..)
                    )
                })
                .count();
            (n, sensed.own_food_share)
        };

        let (walked, _) = moves(&base, 25.0);
        assert_eq!(walked, 200, "speed 255 moves every tick without the switch");

        let stay = WorldConfig {
            foragers_stay_on_food: true,
            ..base.clone()
        };
        let (stayed, share) = moves(&stay, 25.0);
        assert!((share - 25.0 / 27.0).abs() < 1e-4, "own share {share}");
        assert!(
            stayed < 40,
            "walked off a 25-decay corpse {stayed}/200 times with a trace of 2 the best alternative"
        );
        let (left, share) = moves(&stay, 0.0);
        assert_eq!(share, 0.0);
        assert_eq!(left, 200, "an eaten-out tile should not hold the forager");
    }

    #[test]
    fn offspring_scatter_places_a_child_beyond_the_parents_own_tile_ring() {
        // spec.md gene 22, offspring_scatter: "distance from parent at which
        // offspring spawns". The gene was never read — every child budded
        // into an adjacent tile, so a lineage could only ever creep one tile
        // per generation and clusters 128 tiles apart never met.
        let config = WorldConfig {
            grid_width: 32,
            grid_height: 32,
            vent_count: 0,
            ..WorldConfig::default()
        };
        let mut data = [0u8; GENOME_LEN];
        data[genome::SENSE_RADIUS] = 255;
        data[genome::OFFSPRING_SCATTER] = 255;
        let far = Genome::new(data).decode(&config);
        data[genome::OFFSPRING_SCATTER] = 0;
        let near = Genome::new(data).decode(&config);

        assert!(
            mapped_offspring_scatter(&far) > 1,
            "a maxed offspring_scatter still only reaches {} tile(s)",
            mapped_offspring_scatter(&far)
        );
        assert_eq!(mapped_offspring_scatter(&near), 1);

        // And the gate really draws from the wider set.
        let mut world = World::new(&config);
        let id = world.spawn_cell(Cell::new(Genome::new(data), 200.0, (16, 16)));
        world.set_current_tile_cell_id(16, 16, id);
        let sensed = sense(world.get_cell(id), &far, &world, &config);
        assert!(
            sensed.empty_scatter.len() > sensed.empty_adjacent.len(),
            "scatter set {} is no wider than the adjacent set {}",
            sensed.empty_scatter.len(),
            sensed.empty_adjacent.len()
        );
        assert!(
            sensed
                .empty_scatter
                .iter()
                .any(|&(x, y)| toroidal_dist(16, 16, x, y, 32, 32) > 1),
            "no scatter target sits further than one tile from the parent"
        );
    }

    #[test]
    fn resolve_all_move_updates_position_for_next_tick() {
        let config = small_config();
        let cell = make_cell_at(5, 5, make_genome(128), 50.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![cell]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        resolve_all(
            &[(ids[0], Action::Move(6, 5))],
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );
        assert_eq!(world.get_cell(ids[0]).position, (6, 5));

        // Next tick: an Idle cell must stay where it moved to, not snap back.
        world.swap_buffers();
        world.prepare_next();
        resolve_all(
            &[(ids[0], Action::Idle)],
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );
        assert_eq!(world.next_tile(6, 5).cell_id, ids[0]);
        assert_eq!(world.next_tile(5, 5).cell_id, 0);
        assert_eq!(world.get_cell(ids[0]).position, (6, 5));
    }

    #[test]
    fn resolve_all_reproduce_creates_child() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::OFFSPRING_ENERGY_SHARE] = 128;
        data[genome::REPRODUCTION_COOLDOWN] = 50;
        let genome = Genome::new(data);
        let cell = make_cell_at(5, 5, genome, 100.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![cell]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let actions = vec![(ids[0], Action::Reproduce(6, 5))];
        resolve_all(
            &actions,
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        // Parent should stay at original position
        assert_eq!(world.next_tile(5, 5).cell_id, ids[0]);
        // Child should be placed at target
        let child_id = world.next_tile(6, 5).cell_id;
        assert!(child_id != 0, "child should be placed at target tile");
        assert!(child_id != ids[0], "child should have a different id");

        // Parent energy should be reduced
        let parent = world.get_cell(ids[0]);
        assert!(parent.energy < 100.0, "parent energy should decrease");
        assert!(parent.cooldown_remaining > 0, "cooldown should be set");
    }

    #[test]
    fn resolve_all_move_does_not_overwrite_newborn() {
        let config = small_config();
        let mut data = [128u8; GENOME_LEN];
        data[genome::OFFSPRING_ENERGY_SHARE] = 128;
        let parent = make_cell_at(5, 5, Genome::new(data), 100.0);
        let mover = make_cell_at(7, 5, make_genome(128), 50.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![parent, mover]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Both target (6, 5), which is empty in the current grid.
        let actions = vec![
            (ids[0], Action::Reproduce(6, 5)),
            (ids[1], Action::Move(6, 5)),
        ];
        resolve_all(
            &actions,
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        let child_id = world.next_tile(6, 5).cell_id;
        assert!(
            child_id != 0 && child_id != ids[1],
            "newborn must keep its tile"
        );
        assert_eq!(
            world.next_tile(7, 5).cell_id,
            ids[1],
            "blocked mover stays put"
        );
        assert_eq!(world.get_cell(ids[1]).position, (7, 5));
    }

    #[test]
    fn resolve_all_attack_deals_damage() {
        let config = small_config();
        let mut data_a = [0u8; GENOME_LEN];
        data_a[genome::ATTACK_POWER] = 200;
        data_a[genome::ARMOR] = 50;
        let attacker = make_cell_at(5, 5, Genome::new(data_a), 100.0);

        let mut data_d = [0u8; GENOME_LEN];
        data_d[genome::ATTACK_POWER] = 100;
        data_d[genome::ARMOR] = 50;
        let defender = make_cell_at(6, 5, Genome::new(data_d), 100.0);

        let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, defender]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let actions = vec![(ids[0], Action::Attack(ids[1])), (ids[1], Action::Idle)];
        resolve_all(
            &actions,
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        let a = world.get_cell(ids[0]);
        let d = world.get_cell(ids[1]);
        assert!(a.energy < 100.0, "attacker should take retaliation damage");
        assert!(d.energy < 100.0, "defender should take attack damage");
        // Both placed at original positions
        assert_eq!(world.next_tile(5, 5).cell_id, ids[0]);
        assert_eq!(world.next_tile(6, 5).cell_id, ids[1]);
    }

    #[test]
    fn phase_offense_modifier_reaches_combat() {
        // B7: resolve_all re-decoded genomes without phase modifiers, so the
        // offense and defense phase groups had no effect on anything.
        use crate::sim::genome::{BASE_GENE_COUNT, PHASE_OFFENSE_MOD};

        let config = small_config();
        let damage_with_phase = |phase: u8, offense_mod: u8| {
            let mut att = [0u8; GENOME_LEN];
            att[genome::ATTACK_POWER] = 200;
            att[BASE_GENE_COUNT + PHASE_OFFENSE_MOD] = offense_mod;
            let mut attacker = Cell::new(Genome::new(att), 100.0, (5, 5));
            attacker.active_phase = phase;
            let prey = Cell::new(make_genome(0), 5000.0, (6, 5));
            let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, prey]);
            let mut rng = ChaCha8Rng::seed_from_u64(1);
            resolve_all(
                &[(ids[0], Action::Attack(ids[1]))],
                &mut world,
                &config,
                &mut rng,
                &mut DecodeCache::default(),
            );
            5000.0 - world.get_cell(ids[1]).energy
        };

        let base = damage_with_phase(0, 255);
        let boosted = damage_with_phase(1, 255); // slot 0 active, offense x~2
        let suppressed = damage_with_phase(1, 0); // slot 0 active, offense x0

        assert!(
            boosted > base,
            "an offense phase should hit harder: {boosted} vs {base}"
        );
        assert!(
            suppressed < base,
            "a suppressing phase should hit softer: {suppressed} vs {base}"
        );
    }

    #[test]
    fn killing_feeds_the_killer() {
        // Predation has to pay, or no predator can ever cover its upkeep.
        let config = small_config();
        let mut att = [0u8; GENOME_LEN];
        att[genome::ATTACK_POWER] = 255;
        att[genome::PREDATION_EFFICIENCY] = 255;
        let attacker = make_cell_at(5, 5, Genome::new(att), 50.0);
        let prey = make_cell_at(6, 5, make_genome(0), 40.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, prey]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        resolve_all(
            &[(ids[0], Action::Attack(ids[1]))],
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        assert!(!world.get_cell(ids[1]).is_alive(), "prey should die");
        assert!(
            world.get_cell(ids[0]).energy > 50.0,
            "killer should absorb energy, has {}",
            world.get_cell(ids[0]).energy
        );
    }

    /// A kill paid `predation_efficiency` x the victim's whole store, the
    /// gene has no antagonist, and whatever the killer did not take vanished.
    /// `max_predation_efficiency` bounds the gene; with `corpses_keep_energy`
    /// the uneaten part stays on the victim's tile as decay.
    #[test]
    fn a_capped_kill_pays_the_killer_its_share_and_leaves_the_rest_in_the_body() {
        let config = WorldConfig {
            max_predation_efficiency: 0.25,
            corpses_keep_energy: true,
            initial_decay_matter: 0.0,
            ..small_config()
        };
        let mut att = [0u8; GENOME_LEN];
        att[genome::ATTACK_POWER] = 255;
        att[genome::PREDATION_EFFICIENCY] = 255;
        let attacker = make_cell_at(5, 5, Genome::new(att), 50.0);
        let prey = make_cell_at(6, 5, make_genome(0), 40.0);
        let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, prey]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        resolve_all(
            &[(ids[0], Action::Attack(ids[1]))],
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        assert!(!world.get_cell(ids[1]).is_alive(), "prey should die");
        // The prey cannot hit back (attack 0), so the killer's gain is the meal.
        let gained = world.get_cell(ids[0]).energy - 50.0;
        assert!(
            (gained - 0.25 * 40.0).abs() < 1e-3,
            "a maxed gene under a 0.25 cap should pay 10 of 40, paid {gained}"
        );
        let left = world.next_tile(6, 5).decay_energy;
        assert!(
            (left - 0.75 * 40.0).abs() < 1e-3,
            "the 30 the killer did not take should lie on the corpse tile, found {left}"
        );
    }

    #[test]
    fn surviving_an_attack_feeds_nobody() {
        let config = small_config();
        let mut att = [0u8; GENOME_LEN];
        att[genome::ATTACK_POWER] = 60;
        att[genome::PREDATION_EFFICIENCY] = 255;
        let attacker = make_cell_at(5, 5, Genome::new(att), 50.0);
        let mut def = [0u8; GENOME_LEN];
        def[genome::ARMOR] = 255; // shrugs it off
        let prey = Cell::new(Genome::new(def), 200.0, (6, 5));
        let (mut world, ids) = setup_world_with_cells(&config, vec![attacker, prey]);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        resolve_all(
            &[(ids[0], Action::Attack(ids[1]))],
            &mut world,
            &config,
            &mut rng,
            &mut DecodeCache::default(),
        );

        assert!(world.get_cell(ids[1]).is_alive());
        assert!(world.get_cell(ids[0]).energy <= 50.0, "no kill, no meal");
    }

    #[test]
    fn kin_recognition_survives_top_n_gating() {
        // Aggression and precision are mid-range, so neither is among the
        // cell's dozen strongest genes and both decode to ~0.1x. A sibling
        // 5% away must still read as kin, not as a threat.
        let config = small_config();
        let mut data = [250u8; GENOME_LEN];
        data[genome::AGGRESSION_TRIGGER] = 120;
        data[genome::KIN_RECOGNITION_PRECISION] = 120;
        let genome = Genome::new(data);
        let genes = genome.decode(&config);
        let gated_trigger = genes.get(genome::AGGRESSION_TRIGGER)
            * (0.5 + 0.5 * genes.get(genome::KIN_RECOGNITION_PRECISION));

        // A sibling: 20 of 46 genes shifted by 41.
        let mut sibling_data = data;
        for b in sibling_data.iter_mut().take(20) {
            *b -= 41;
        }
        let sibling = Genome::new(sibling_data);
        let distance = genetic_distance(&genome, &sibling);
        assert!(
            distance > gated_trigger,
            "precondition: gated trigger {gated_trigger} must be under sibling distance {distance}"
        );

        let cell = make_cell_at(5, 5, genome, 50.0);
        let neighbour = make_cell_at(6, 5, sibling, 50.0);
        let (world, ids) = setup_world_with_cells(&config, vec![cell, neighbour]);

        let sensed = sense(world.get_cell(ids[0]), &genes, &world, &config);
        assert_eq!(sensed.kin_count, 1, "sibling should read as kin");
        assert_eq!(sensed.threat_count, 0);
    }

    #[test]
    fn resolve_all_order_independent() {
        // Same actions in different order should produce identical next-grid state.
        let config = small_config();

        // Set up: one cell reproduces, one moves, one idles
        let cell_a = make_cell_at(3, 3, make_genome(100), 100.0);
        let cell_b = make_cell_at(7, 7, make_genome(200), 50.0);
        let cell_c = make_cell_at(10, 10, make_genome(50), 30.0);

        // Run with order A
        let (mut world_a, ids_a) = setup_world_with_cells(
            &config,
            vec![cell_a.clone(), cell_b.clone(), cell_c.clone()],
        );
        let mut rng_a = ChaCha8Rng::seed_from_u64(99);
        let actions_a = vec![
            (ids_a[0], Action::Reproduce(4, 3)),
            (ids_a[1], Action::Move(8, 7)),
            (ids_a[2], Action::Idle),
        ];
        resolve_all(
            &actions_a,
            &mut world_a,
            &config,
            &mut rng_a,
            &mut DecodeCache::default(),
        );

        // Run with reversed order
        let (mut world_b, ids_b) = setup_world_with_cells(
            &config,
            vec![cell_a.clone(), cell_b.clone(), cell_c.clone()],
        );
        let mut rng_b = ChaCha8Rng::seed_from_u64(99);
        let actions_b = vec![
            (ids_b[2], Action::Idle),
            (ids_b[1], Action::Move(8, 7)),
            (ids_b[0], Action::Reproduce(4, 3)),
        ];
        resolve_all(
            &actions_b,
            &mut world_b,
            &config,
            &mut rng_b,
            &mut DecodeCache::default(),
        );

        // Compare next-grid state: same cells at same positions
        assert_eq!(
            world_a.next_tile(3, 3).cell_id,
            world_b.next_tile(3, 3).cell_id
        );
        assert_eq!(
            world_a.next_tile(4, 3).cell_id,
            world_b.next_tile(4, 3).cell_id
        );
        assert_eq!(
            world_a.next_tile(8, 7).cell_id,
            world_b.next_tile(8, 7).cell_id
        );
        assert_eq!(
            world_a.next_tile(10, 10).cell_id,
            world_b.next_tile(10, 10).cell_id
        );

        // Parent energies should match
        let pa = world_a.get_cell(ids_a[0]);
        let pb = world_b.get_cell(ids_b[0]);
        assert!(
            (pa.energy - pb.energy).abs() < 1e-6,
            "parent energy should be identical regardless of action order"
        );
    }
}
