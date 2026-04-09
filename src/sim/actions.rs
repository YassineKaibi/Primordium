// Action enum (Move, Attack, Reproduce, Share, Idle),
// decision logic, conflict resolution

use crate::config::WorldConfig;
use crate::sim::cell::Cell;
use crate::sim::genome::{self, BASE_GENE_COUNT, DecodedGenes, Genome};
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
pub fn genetic_distance(a: &Genome, b: &Genome) -> f32 {
    let sum: u32 = (0..BASE_GENE_COUNT)
        .map(|i| (a.gene(i) as i16 - b.gene(i) as i16).unsigned_abs() as u32)
        .sum();
    sum as f32 / (BASE_GENE_COUNT as f32 * 255.0)
}

// ── SenseResult ────────────────────────────────────────────────────

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
}

// ── Gene value mapping helpers ─────────────────────────────────────

/// Map reproduction_threshold gene to absolute energy value.
fn mapped_reproduction_threshold(genes: &DecodedGenes) -> f32 {
    let energy_cap = genes.get(genome::ENERGY_STORAGE_CAP) * 255.0;
    genes.get(genome::REPRODUCTION_THRESHOLD) * energy_cap
}

/// Map maturity_age gene to tick count.
fn mapped_maturity_age(genes: &DecodedGenes) -> u32 {
    (genes.get(genome::MATURITY_AGE) * 1000.0) as u32
}

/// Map attack_range gene to tile distance (1-3).
fn mapped_attack_range(genes: &DecodedGenes) -> u16 {
    let raw = (genes.get(genome::ATTACK_RANGE) * 3.0).ceil() as u16;
    raw.max(1)
}

/// Map reproduction_cooldown gene to tick count.
fn mapped_reproduction_cooldown(genes: &DecodedGenes) -> u16 {
    (genes.get(genome::REPRODUCTION_COOLDOWN) * 100.0) as u16
}

// ── Decision logic ─────────────────────────────────────────────────

/// Choose an action for a cell based on priority gates.
///
/// Priority: Reproduce > Attack > Flee > Move > Share > Idle.
/// First passing gate wins.
pub fn decide(
    cell: &Cell,
    genes: &DecodedGenes,
    sense: &SenseResult,
    rng: &mut impl rand::Rng,
) -> Action {
    // Gate 1: Reproduce
    let repro_threshold = mapped_reproduction_threshold(genes);
    let maturity = mapped_maturity_age(genes);
    if cell.energy > repro_threshold
        && cell.cooldown_remaining == 0
        && cell.age >= maturity
        && !sense.empty_adjacent.is_empty()
    {
        let idx = rng.gen_range(0..sense.empty_adjacent.len());
        let (tx, ty) = sense.empty_adjacent[idx];
        return Action::Reproduce(tx, ty);
    }

    // Gate 2: Attack
    let attack_range = mapped_attack_range(genes);
    if let Some((_tx, _ty, target_id, dist)) = sense.nearest_threat
        && dist <= attack_range
    {
        return Action::Attack(target_id);
    }

    // Gate 3: Flee
    if sense.nearest_threat.is_some()
        && genes.get(genome::FLEE_RESPONSE) > 0.0
        && let Some(flee_tile) = flee_direction(cell, sense)
    {
        return Action::Flee(flee_tile.0, flee_tile.1);
    }

    // Gate 4: Move
    if rng.r#gen::<f32>() < genes.get(genome::SPEED)
        && let Some((mx, my)) = compute_move_target(cell, genes, sense, rng)
    {
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

/// Find the flee direction: opposite vector from nearest threat, resolved to an adjacent tile.
fn flee_direction(cell: &Cell, sense: &SenseResult) -> Option<(u16, u16)> {
    let (tx, ty, _, _) = sense.nearest_threat?;
    let (cx, cy) = cell.position;
    // Vector from threat to cell (away from threat)
    let dx = cx as i32 - tx as i32;
    let dy = cy as i32 - ty as i32;
    // Normalize to -1, 0, 1
    let ndx = dx.signum();
    let ndy = dy.signum();

    // Check if opposite-direction adjacent tile is empty
    let _target = (cx as i32 + ndx, cy as i32 + ndy);
    // Find this in empty_adjacent (they're already wrapped)
    // We need to check if any empty adjacent tile is in the flee direction
    sense
        .empty_adjacent
        .iter()
        .copied()
        .find(|&(ex, ey)| {
            let edx = (ex as i32 - cx as i32).signum();
            let edy = (ey as i32 - cy as i32).signum();
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
    let noise_angle = rng.r#gen::<f32>() * direction_noise * std::f32::consts::PI;
    let angle = direction_bias * 2.0 * std::f32::consts::PI;
    let combined_angle = angle + noise_angle;

    let mut hx = combined_angle.cos();
    let mut hy = combined_angle.sin();

    // 2. Chemotaxis: pull toward pheromone gradient
    let chemotaxis = genes.get(genome::CHEMOTAXIS_STRENGTH);
    hx += sense.pheromone_gradient.0 as f32 * chemotaxis;
    hy += sense.pheromone_gradient.1 as f32 * chemotaxis;

    let cx = cell.position.0;
    let cy = cell.position.1;

    // 3. Pack affinity: pull toward nearest kin
    if let Some(nearest_kin) = sense.nearest_kin
        && nearest_kin.2 != 0
    {
        let pack = genes.get(genome::PACK_AFFINITY);
        hx += (nearest_kin.0 as f32 - cx as f32) * pack;
        hy += (nearest_kin.1 as f32 - cy as f32) * pack;
    }

    // 4. Memory: momentum from previous tick direction
    hx += cell.memory_dir.0 as f32 * 0.5;
    hy += cell.memory_dir.1 as f32 * 0.5;

    // 5. Zero-vector fallback: pick random tile
    if hx.abs() < f32::EPSILON && hy.abs() < f32::EPSILON {
        let idx = rng.gen_range(0..sense.empty_adjacent.len());
        return Some(sense.empty_adjacent[idx]);
    }

    // 6. Score each empty adjacent tile by dot product with heading
    let mut best_tile = sense.empty_adjacent[0];
    let mut best_score = f32::NEG_INFINITY;

    for &(tx, ty) in &sense.empty_adjacent {
        let dx = tx as f32 - cx as f32;
        let dy = ty as f32 - cy as f32;
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
pub fn sense(cell: &Cell, genes: &DecodedGenes, world: &World) -> SenseResult {
    let (cx, cy) = cell.position;
    let radius = mapped_sense_radius(genes);

    let local_tile = TileSnapshot::from_tile(world.current_tile(cx, cy));

    // Kin recognition: precision scales the effective aggression trigger.
    // Low precision widens the "hostile" band.
    let precision = genes.get(genome::KIN_RECOGNITION_PRECISION);
    let aggression = genes.get(genome::AGGRESSION_TRIGGER);
    let effective_trigger = aggression * (0.5 + 0.5 * precision);

    // Determine which acquisition gene is strongest (for food detection).
    let photo = genes.get(genome::PHOTOSYNTHESIS_RATE);
    let thermo = genes.get(genome::THERMOSYNTHESIS_RATE);
    let scavenge = genes.get(genome::SCAVENGE_ABILITY);

    let neighbors = world.neighbors_in_radius(cx, cy, radius);

    let mut nearest_food: Option<(u16, u16)> = None;
    let mut nearest_food_dist = i32::MAX;
    let mut nearest_threat: Option<(u16, u16, u32, u16)> = None;
    let mut nearest_threat_dist = i32::MAX;
    let mut nearest_kin: Option<(u16, u16, u32)> = None;
    let mut nearest_kin_dist = i32::MAX;
    let mut kin_count: u32 = 0;
    let mut threat_count: u32 = 0;
    let mut neighbor_count: u32 = 0;
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
                is_near_vent(wx, wy, world)
            };

            if is_food {
                food_nearby = true;
                let tile_dist = toroidal_dist(cx, cy, wx, wy, world.width, world.height);
                if tile_dist < nearest_food_dist {
                    nearest_food_dist = tile_dist;
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
        is_near_vent(cx, cy, world)
    };
    if own_is_food {
        food_nearby = true;
    }

    // Scale pheromone gradient by signal_sensitivity
    let sensitivity = genes.get(genome::SIGNAL_SENSITIVITY);
    let pheromone_gradient = if best_pheromone > 0.0 && sensitivity > 0.0 {
        (
            (pheromone_dir.0 as f32 * sensitivity).round() as i8,
            (pheromone_dir.1 as f32 * sensitivity).round() as i8,
        )
    } else {
        (0, 0)
    };

    // Collect empty adjacent tiles (radius 1 only, for placement actions)
    let mut empty_adjacent = Vec::new();
    for dy in -1..=1_i32 {
        for dx in -1..=1_i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (wx, wy) = world.wrap(cx as i32 + dx, cy as i32 + dy);
            if world.current_tile(wx, wy).cell_id == 0 {
                empty_adjacent.push((wx, wy));
            }
        }
    }

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
    }
}

/// Check if a tile is near a thermal vent (within 1 tile of a vent on the bottom row).
fn is_near_vent(x: u16, y: u16, world: &World) -> bool {
    let bottom = world.height - 1;
    if (y as u32) < bottom.saturating_sub(1) {
        return false;
    }
    world
        .vent_positions
        .iter()
        .any(|&vx| (x as i32 - vx as i32).unsigned_abs() <= 1)
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
/// - Child energy = `parent_energy * offspring_energy_share`
/// - Parent energy is reduced by child's energy
/// - Parent cooldown set from `reproduction_cooldown` gene
/// - Child is a fresh Cell at `target_pos` with age 0, no venom, no cooldown
pub fn resolve_reproduction(
    parent: &Cell,
    genes: &DecodedGenes,
    target_pos: (u16, u16),
    rng: &mut impl rand::Rng,
) -> ReproductionOutcome {
    let mut child_genome = parent.genome.clone();
    child_genome.mutate(rng);

    let child_energy = parent.energy * genes.get(genome::OFFSPRING_ENERGY_SHARE);
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
pub fn resolve_share(
    donor: &Cell,
    donor_genes: &DecodedGenes,
    recipient: &Cell,
) -> ShareOutcome {
    let shared_energy =
        (donor_genes.get(genome::RESOURCE_SHARING) * donor.energy * 0.1).max(0.0);
    let donor_energy = donor.energy - shared_energy;
    let recipient_energy = recipient.energy + shared_energy;
    ShareOutcome {
        donor_energy,
        recipient_energy,
    }
}

// ── Placement helper ─────────────────────────────────────────────

/// Check whether a cell has already been placed in the next grid.
///
/// A cell is "placed" if the next-grid tile at its original position
/// already has its cell_id written.
fn is_placed(world: &World, cell_id: u32, pos: (u16, u16)) -> bool {
    world.next_tile(pos.0, pos.1).cell_id == cell_id
}

/// Copy a cell into the next grid at the given position, setting the
/// tile's cell_id.
fn place_cell(world: &mut World, cell_id: u32, pos: (u16, u16)) {
    let tile = world.next_tile_mut(pos.0, pos.1);
    tile.cell_id = cell_id;
}

// ── resolve_all orchestrator ─────────────────────────────────────

/// Process all actions simultaneously, writing results to the next grid.
///
/// Actions are resolved in priority order across ALL cells:
/// Phase 1: Reproduce — Phase 2: Attack — Phase 3: Flee/Move —
/// Phase 4: Share — Phase 5: Idle.
///
/// Placement tracking: once a cell is placed in the next grid by any
/// phase, later phases skip it (no double-placement).
pub fn resolve_all(
    actions: &[(u32, Action)],
    world: &mut World,
    config: &WorldConfig,
    rng: &mut impl rand::Rng,
) {
    let tick = world.tick;

    // ── Phase 1: Reproduce ───────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Reproduce(tx, ty) = *action {
            // Target tile must still be empty in next grid
            if world.next_tile(tx, ty).cell_id != 0 {
                // Another reproduction already claimed this tile
                place_cell(world, cell_id, world.get_cell(cell_id).position);
                continue;
            }

            let parent = world.get_cell(cell_id);
            let genes = parent.genome.decode(config);
            let pos = parent.position;

            let outcome = resolve_reproduction(parent, &genes, (tx, ty), rng);

            // Place child in next grid
            let child_id = world.spawn_cell(outcome.child);
            place_cell(world, child_id, (tx, ty));

            // Update parent state and place at original position
            let parent_mut = world.get_cell_mut(cell_id);
            parent_mut.energy = outcome.parent_energy;
            parent_mut.cooldown_remaining = outcome.parent_cooldown;
            place_cell(world, cell_id, pos);
        }
    }

    // ── Phase 2: Attack ──────────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Attack(target_id) = *action {
            let attacker_cell = world.get_cell(cell_id);
            let attacker_genes = attacker_cell.genome.decode(config);
            let attacker_pos = attacker_cell.position;

            let defender_cell = world.get_cell(target_id);
            let defender_genes = defender_cell.genome.decode(config);
            let defender_pos = defender_cell.position;

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
            let a = world.get_cell_mut(cell_id);
            a.energy -= outcome.damage_to_attacker;
            a.last_damage_tick = tick as u32;

            // Apply damage + venom to defender
            let d = world.get_cell_mut(target_id);
            d.energy -= outcome.damage_to_defender;
            d.last_damage_tick = tick as u32;
            if outcome.venom_ticks > 0 {
                d.venom_ticks = outcome.venom_ticks;
                d.venom_damage = outcome.venom_damage;
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

        let genes = cell.genome.decode(config);
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
            MoveOutcome::Wins(cid, x, y) => place_cell(world, cid, (x, y)),
            MoveOutcome::Loses(cid, x, y) => place_cell(world, cid, (x, y)),
        }
    }

    // ── Phase 4: Share ───────────────────────────────────────────
    for &(cell_id, ref action) in actions {
        if let Action::Share(target_id) = *action {
            let donor = world.get_cell(cell_id);
            let donor_pos = donor.position;
            let recipient = world.get_cell(target_id);
            let recipient_pos = recipient.position;

            let donor_genes = donor.genome.decode(config);
            let outcome = resolve_share(donor, &donor_genes, recipient);

            // Apply energy changes
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
        let result = sense(&cell_a, &genes, &world);

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
        let result = sense(&cell_a, &genes, &world);

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
        let result = sense(&cell_a, &genes, &world);

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

        let result = sense(&cell, &genes, &world);

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
        let result = sense(&cell, &genes, &world);

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
        let result = sense(&cell, &genes, &world);

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

        let action = decide(&cell, &genes, &sense, &mut rng);
        assert!(
            matches!(action, Action::Reproduce(_, _)),
            "should reproduce with high energy: got {:?}",
            action
        );
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

        let action = decide(&cell, &genes, &sense, &mut rng);
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
        let action = decide(&cell, &genes, &sense, &mut rng);
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
        let action = decide(&cell, &genes, &sense, &mut rng);
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

        let action = decide(&cell, &genes, &sense, &mut rng);
        assert!(
            matches!(action, Action::Move(_, _)),
            "should move with high speed and no threats: got {:?}",
            action
        );
    }

    // ── resolve_movement_conflicts tests ──────────────────────────────

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

        let action = decide(&cell, &genes, &sense, &mut rng);
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

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &mut rng);

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

    #[test]
    fn reproduction_child_placed_at_target() {
        let config = small_config();
        let data = [128u8; GENOME_LEN];
        let genome = Genome::new(data);
        let parent = make_cell_at(5, 5, genome.clone(), 100.0);
        let genes = genome.decode(&config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &mut rng);

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

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &mut rng);
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

        let outcome = resolve_reproduction(&parent, &genes, (6, 5), &mut rng);

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
    fn setup_world_with_cells(
        config: &WorldConfig,
        cells: Vec<Cell>,
    ) -> (World, Vec<u32>) {
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
        resolve_all(&actions, &mut world, &config, &mut rng);

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
        resolve_all(&actions, &mut world, &config, &mut rng);

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
        resolve_all(&actions, &mut world, &config, &mut rng);

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
        resolve_all(&actions, &mut world, &config, &mut rng);

        let a = world.get_cell(ids[0]);
        let d = world.get_cell(ids[1]);
        assert!(a.energy < 100.0, "attacker should take retaliation damage");
        assert!(d.energy < 100.0, "defender should take attack damage");
        // Both placed at original positions
        assert_eq!(world.next_tile(5, 5).cell_id, ids[0]);
        assert_eq!(world.next_tile(6, 5).cell_id, ids[1]);
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
        let (mut world_a, ids_a) =
            setup_world_with_cells(&config, vec![cell_a.clone(), cell_b.clone(), cell_c.clone()]);
        let mut rng_a = ChaCha8Rng::seed_from_u64(99);
        let actions_a = vec![
            (ids_a[0], Action::Reproduce(4, 3)),
            (ids_a[1], Action::Move(8, 7)),
            (ids_a[2], Action::Idle),
        ];
        resolve_all(&actions_a, &mut world_a, &config, &mut rng_a);

        // Run with reversed order
        let (mut world_b, ids_b) =
            setup_world_with_cells(&config, vec![cell_a.clone(), cell_b.clone(), cell_c.clone()]);
        let mut rng_b = ChaCha8Rng::seed_from_u64(99);
        let actions_b = vec![
            (ids_b[2], Action::Idle),
            (ids_b[1], Action::Move(8, 7)),
            (ids_b[0], Action::Reproduce(4, 3)),
        ];
        resolve_all(&actions_b, &mut world_b, &config, &mut rng_b);

        // Compare next-grid state: same cells at same positions
        assert_eq!(world_a.next_tile(3, 3).cell_id, world_b.next_tile(3, 3).cell_id);
        assert_eq!(world_a.next_tile(4, 3).cell_id, world_b.next_tile(4, 3).cell_id);
        assert_eq!(world_a.next_tile(8, 7).cell_id, world_b.next_tile(8, 7).cell_id);
        assert_eq!(world_a.next_tile(10, 10).cell_id, world_b.next_tile(10, 10).cell_id);

        // Parent energies should match
        let pa = world_a.get_cell(ids_a[0]);
        let pb = world_b.get_cell(ids_b[0]);
        assert!(
            (pa.energy - pb.energy).abs() < 1e-6,
            "parent energy should be identical regardless of action order"
        );
    }
}
