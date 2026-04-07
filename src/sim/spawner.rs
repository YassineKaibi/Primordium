// Initial seeding strategies, cell creation

use rand::Rng;

use crate::config::{SeedStrategy, WorldConfig};
use crate::sim::cell::Cell;
use crate::sim::genome::{
    GENOME_LEN, Genome, PHOTOSYNTHESIS_RATE, PREDATION_EFFICIENCY, SCAVENGE_ABILITY,
    THERMOSYNTHESIS_RATE,
};
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
pub fn random_genome(rng: &mut impl Rng, min_viable_acquisition: u8) -> Genome {
    let mut data = [0u8; GENOME_LEN];
    rng.fill(&mut data[..]);

    if min_viable_acquisition > 0 {
        let max_acq = ACQUISITION_GENES
            .iter()
            .map(|&i| data[i])
            .max()
            .unwrap_or(0);

        if max_acq < min_viable_acquisition {
            // Boost the highest acquisition gene to the floor
            let best_idx = ACQUISITION_GENES
                .iter()
                .copied()
                .max_by_key(|&i| data[i])
                .unwrap_or(PHOTOSYNTHESIS_RATE);
            data[best_idx] = min_viable_acquisition;
        }
    }

    Genome::new(data)
}

// ── Starting energy ───────────────────────────────────────────────

/// Calculate starting energy scaled to genome viability.
///
/// `viability = max(effective acquisition genes) - metabolic_cost`
/// `starting_energy = base + (viability / max_viability) * bonus`
///
/// The `max_viability` denominator is 1.0 (a perfect single-gene
/// specialist with zero cost). Viability is clamped to [0, 1].
pub fn starting_energy(genome: &Genome, config: &WorldConfig) -> f32 {
    let decoded = genome.decode(config);

    let max_acquisition = ACQUISITION_GENES
        .iter()
        .map(|&i| decoded.get(i))
        .fold(0.0_f32, f32::max);

    let cost = crate::sim::energy::metabolic_cost(&decoded, 128, config);

    // Normalize cost to the same 0..1 scale as acquisition genes.
    // 46 genes each contributing gene^exponent at max=1.0
    let max_possible_cost =
        crate::sim::genome::BASE_GENE_COUNT as f32 * 1.0_f32.powf(config.metabolic_cost_exponent);
    let cost_norm = (cost / max_possible_cost).min(1.0);

    let viability = (max_acquisition - cost_norm).clamp(0.0, 1.0);

    config.base_spawn_energy + viability * config.bonus_spawn_energy
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

        let genome = random_genome(rng, config.min_viable_acquisition);
        let energy = starting_energy(&genome, config);
        let cell = Cell::new(genome, energy, (x, y));
        let cell_id = world.spawn_cell(cell);
        world.set_current_tile_cell_id(x, y, cell_id);
    }
}

// ── Random clusters strategy ──────────────────────────────────────

/// Place clusters on a regular grid. Each cluster has one ancestor genome
/// with mutated descendants scattered within a cluster radius.
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
        let cy = (row_spacing / 2 + row * row_spacing) as i32;

        // Generate ancestor genome
        let ancestor = random_genome(rng, config.min_viable_acquisition);

        for member in 0..cells_per_cluster {
            if placed >= config.initial_cell_count as usize {
                break;
            }

            let genome = if member == 0 {
                ancestor.clone()
            } else {
                let mut descendant = ancestor.clone();
                descendant.mutate(rng);
                descendant
            };

            let (x, y) = find_open_position_in_radius(world, cx, cy, cluster_radius, rng);

            let energy = starting_energy(&genome, config);
            let cell = Cell::new(genome, energy, (x, y));
            let cell_id = world.spawn_cell(cell);
            world.set_current_tile_cell_id(x, y, cell_id);
            placed += 1;
        }
    }
}

/// Find an unoccupied tile within `radius` of (cx, cy), respecting toroidal wrapping.
/// Tries random probing first (fast when sparse), falls back to deterministic scan.
fn find_open_position_in_radius(
    world: &World,
    cx: i32,
    cy: i32,
    radius: usize,
    rng: &mut impl Rng,
) -> (u16, u16) {
    let r = radius as i32;

    // Try random offsets first (fast path for sparse clusters)
    for _ in 0..64 {
        let dx = rng.gen_range(-r..=r);
        let dy = rng.gen_range(-r..=r);
        let (x, y) = world.wrap(cx + dx, cy + dy);
        if world.current_tile(x, y).cell_id == 0 {
            return (x, y);
        }
    }

    // Fallback: scan all tiles in the radius
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = world.wrap(cx + dx, cy + dy);
            if world.current_tile(x, y).cell_id == 0 {
                return (x, y);
            }
        }
    }

    // Cluster area is full — place at center
    world.wrap(cx, cy)
}

// ── Dispatcher ────────────────────────────────────────────────────

/// Seed the world with initial cells according to the configured strategy.
pub fn seed_world(world: &mut World, config: &WorldConfig, rng: &mut impl Rng) {
    match config.initial_genome_strategy {
        SeedStrategy::RandomUniform => seed_random_uniform(world, config, rng),
        SeedStrategy::RandomClusters => seed_random_clusters(world, config, rng),
        SeedStrategy::PresetArchetypes => {
            // Planned future extension — fall back to clusters
            seed_random_clusters(world, config, rng);
        }
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

    // ── random_genome tests ────────────────────────────────────────

    #[test]
    fn biased_genome_has_viable_acquisition() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..100 {
            let g = random_genome(&mut rng, 40);
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
            let g = random_genome(&mut rng, 0);
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
        let g1 = random_genome(&mut ChaCha8Rng::seed_from_u64(42), 40);
        let g2 = random_genome(&mut ChaCha8Rng::seed_from_u64(42), 40);
        assert_eq!(g1.data, g2.data);
    }

    // ── starting_energy tests ──────────────────────────────────────

    #[test]
    fn starting_energy_at_least_base() {
        let config = default_config();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        for _ in 0..50 {
            let g = random_genome(&mut rng, config.min_viable_acquisition);
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
