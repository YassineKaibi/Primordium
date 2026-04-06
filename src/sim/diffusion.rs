// Generic field diffusion with per-layer config

use crate::config::WorldConfig;
use crate::sim::world::{Tile, World};

// ── Diffusion configuration ────────────────────────────────────────

/// Which neighbors participate in spreading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Neighborhood {
    /// 8 neighbors (includes diagonals).
    Moore,
    /// 4 neighbors (cardinal directions only).
    VonNeumann,
}

/// Per-layer diffusion parameters.
#[derive(Debug, Clone)]
pub struct DiffusionConfig {
    /// Fraction of value lost per tick (evaporation).
    pub decay_rate: f32,
    /// Fraction of value spread to neighbors per tick.
    pub spread_rate: f32,
    /// Which neighbor topology to use.
    pub neighborhood: Neighborhood,
}

// ── Core diffusion function ────────────────────────────────────────

/// Diffuse a scalar field from `src` into `dst` on a toroidal grid.
///
/// Formula per cell:
///   new = value * (1 - decay_rate - spread_rate)
///       + sum(neighbor_values) * (spread_rate / neighbor_count)
///
/// `src` and `dst` must both have length `width * height`.
pub fn diffuse_layer(
    src: &[f32],
    dst: &mut [f32],
    width: u32,
    height: u32,
    config: &DiffusionConfig,
) {
    let w = width as i32;
    let h = height as i32;
    let retain = 1.0 - config.decay_rate - config.spread_rate;

    let (offsets, neighbor_count): (&[(i32, i32)], f32) = match config.neighborhood {
        Neighborhood::Moore => (
            &[
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ],
            8.0,
        ),
        Neighborhood::VonNeumann => (&[(0, -1), (-1, 0), (1, 0), (0, 1)], 4.0),
    };

    let spread_per_neighbor = config.spread_rate / neighbor_count;

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let mut neighbor_sum = 0.0;

            for &(dx, dy) in offsets {
                let nx = (x as i32 + dx).rem_euclid(w) as u32;
                let ny = (y as i32 + dy).rem_euclid(h) as u32;
                neighbor_sum += src[(ny * width + nx) as usize];
            }

            dst[idx] = src[idx] * retain + neighbor_sum * spread_per_neighbor;
        }
    }
}

// ── Decay matter fade ──────────────────────────────────────────────

/// Reduce decay energy on all tiles by multiplying by `(1.0 - rate)`.
pub fn fade_decay(tiles: &mut [Tile], rate: f32) {
    let factor = 1.0 - rate;
    for tile in tiles.iter_mut() {
        tile.decay_energy *= factor;
    }
}

// ── Orchestration ──────────────────────────────────────────────────

/// Run all three diffusion passes (pheromone, toxin, temperature) plus
/// decay fade, reusing the two float buffers from World.
pub fn run_diffusion_phase(world: &mut World, config: &WorldConfig) {
    let width = world.width;
    let height = world.height;
    let total = (width * height) as usize;

    // ── Pheromone (Moore) ──────────────────────────────────────
    let pheromone_config = DiffusionConfig {
        decay_rate: config.pheromone_decay,
        spread_rate: config.pheromone_diffusion,
        neighborhood: Neighborhood::Moore,
    };

    for i in 0..total {
        world.diffusion_a[i] = world.current_grid()[i].pheromone;
    }
    diffuse_layer(
        &world.diffusion_a.clone(),
        &mut world.diffusion_b,
        width,
        height,
        &pheromone_config,
    );
    for i in 0..total {
        world.current_grid_mut()[i].pheromone = world.diffusion_b[i];
    }

    // ── Toxin (Von Neumann) ────────────────────────────────────
    let toxin_config = DiffusionConfig {
        decay_rate: config.toxin_decay,
        spread_rate: config.toxin_diffusion,
        neighborhood: Neighborhood::VonNeumann,
    };

    for i in 0..total {
        world.diffusion_a[i] = world.current_grid()[i].toxin;
    }
    diffuse_layer(
        &world.diffusion_a.clone(),
        &mut world.diffusion_b,
        width,
        height,
        &toxin_config,
    );
    for i in 0..total {
        world.current_grid_mut()[i].toxin = world.diffusion_b[i];
    }

    // ── Temperature (Moore, u8 -> f32 -> u8) ───────────────────
    let temp_config = DiffusionConfig {
        decay_rate: config.temperature_decay,
        spread_rate: config.temperature_diffusion,
        neighborhood: Neighborhood::Moore,
    };

    for i in 0..total {
        world.diffusion_a[i] = world.current_grid()[i].temperature as f32;
    }
    diffuse_layer(
        &world.diffusion_a.clone(),
        &mut world.diffusion_b,
        width,
        height,
        &temp_config,
    );
    for i in 0..total {
        world.current_grid_mut()[i].temperature = world.diffusion_b[i].clamp(0.0, 255.0) as u8;
    }

    // ── Decay fade ─────────────────────────────────────────────
    fade_decay(world.current_grid_mut(), config.decay_rate);
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorldConfig;

    fn make_grid(width: u32, height: u32) -> Vec<f32> {
        vec![0.0; (width * height) as usize]
    }

    fn moore_config(decay: f32, spread: f32) -> DiffusionConfig {
        DiffusionConfig {
            decay_rate: decay,
            spread_rate: spread,
            neighborhood: Neighborhood::Moore,
        }
    }

    fn vn_config(decay: f32, spread: f32) -> DiffusionConfig {
        DiffusionConfig {
            decay_rate: decay,
            spread_rate: spread,
            neighborhood: Neighborhood::VonNeumann,
        }
    }

    // 1. Center cell spreads to neighbors after one diffusion pass
    #[test]
    fn center_cell_spreads_to_neighbors() {
        let w = 8u32;
        let h = 8u32;
        let mut src = make_grid(w, h);
        let mut dst = make_grid(w, h);

        src[(4 * w + 4) as usize] = 100.0;

        let cfg = moore_config(0.0, 0.8);
        diffuse_layer(&src, &mut dst, w, h, &cfg);

        let center = dst[(4 * w + 4) as usize];
        assert!(center > 0.0, "center should retain value, got {center}");
        assert!(center < 100.0, "center should have spread, got {center}");

        let neighbor = dst[(3 * w + 4) as usize];
        assert!(
            neighbor > 0.0,
            "neighbor should receive spread, got {neighbor}"
        );
    }

    // 2. Total field value is approximately conserved
    #[test]
    fn total_value_conserved_without_decay() {
        let w = 16u32;
        let h = 16u32;
        let mut src = make_grid(w, h);
        let mut dst = make_grid(w, h);

        src[0] = 50.0;
        src[100] = 30.0;
        src[200] = 20.0;

        let initial_total: f32 = src.iter().sum();

        let cfg = moore_config(0.0, 0.5);
        diffuse_layer(&src, &mut dst, w, h, &cfg);

        let final_total: f32 = dst.iter().sum();

        let diff = (initial_total - final_total).abs();
        assert!(
            diff < 0.01,
            "total should be conserved: initial={initial_total}, final={final_total}, diff={diff}"
        );
    }

    // 3. Decay matter fades toward zero
    #[test]
    fn decay_matter_fades_toward_zero() {
        let mut tiles = vec![Tile::EMPTY; 4];
        tiles[0].decay_energy = 100.0;
        tiles[1].decay_energy = 50.0;

        for _ in 0..100 {
            fade_decay(&mut tiles, 0.1);
        }

        assert!(
            tiles[0].decay_energy < 0.01,
            "decay should approach zero, got {}",
            tiles[0].decay_energy
        );
        assert!(
            tiles[1].decay_energy < 0.01,
            "decay should approach zero, got {}",
            tiles[1].decay_energy
        );
    }

    // 4. Pheromone decays faster than toxin over N ticks
    #[test]
    fn pheromone_decays_faster_than_toxin() {
        let w = 8u32;
        let h = 8u32;
        let center = (4 * w + 4) as usize;

        let mut pheromone_src = make_grid(w, h);
        let mut pheromone_dst = make_grid(w, h);
        let mut toxin_src = make_grid(w, h);
        let mut toxin_dst = make_grid(w, h);

        pheromone_src[center] = 100.0;
        toxin_src[center] = 100.0;

        let pheromone_cfg = moore_config(0.05, 0.15);
        let toxin_cfg = vn_config(0.005, 0.02);

        for _ in 0..20 {
            diffuse_layer(&pheromone_src, &mut pheromone_dst, w, h, &pheromone_cfg);
            std::mem::swap(&mut pheromone_src, &mut pheromone_dst);

            diffuse_layer(&toxin_src, &mut toxin_dst, w, h, &toxin_cfg);
            std::mem::swap(&mut toxin_src, &mut toxin_dst);
        }

        let pheromone_total: f32 = pheromone_src.iter().sum();
        let toxin_total: f32 = toxin_src.iter().sum();

        assert!(
            pheromone_total < toxin_total,
            "pheromone ({pheromone_total}) should decay faster than toxin ({toxin_total})"
        );
    }

    // 5. Von Neumann only spreads to 4 neighbors (not diagonals)
    #[test]
    fn von_neumann_no_diagonal_spread() {
        let w = 8u32;
        let h = 8u32;
        let mut src = make_grid(w, h);
        let mut dst = make_grid(w, h);

        src[(4 * w + 4) as usize] = 100.0;

        let cfg = vn_config(0.0, 0.8);
        diffuse_layer(&src, &mut dst, w, h, &cfg);

        assert!(dst[(3 * w + 4) as usize] > 0.0, "north should get value");
        assert!(dst[(5 * w + 4) as usize] > 0.0, "south should get value");
        assert!(dst[(4 * w + 3) as usize] > 0.0, "west should get value");
        assert!(dst[(4 * w + 5) as usize] > 0.0, "east should get value");

        assert!(
            dst[(3 * w + 3) as usize] < f32::EPSILON,
            "NW diagonal should be zero"
        );
        assert!(
            dst[(3 * w + 5) as usize] < f32::EPSILON,
            "NE diagonal should be zero"
        );
        assert!(
            dst[(5 * w + 3) as usize] < f32::EPSILON,
            "SW diagonal should be zero"
        );
        assert!(
            dst[(5 * w + 5) as usize] < f32::EPSILON,
            "SE diagonal should be zero"
        );
    }

    // 6. Toroidal wrapping works (corner cell spreads correctly)
    #[test]
    fn toroidal_wrapping_corner_spread() {
        let w = 8u32;
        let h = 8u32;
        let mut src = make_grid(w, h);
        let mut dst = make_grid(w, h);

        src[0] = 100.0;

        let cfg = moore_config(0.0, 0.8);
        diffuse_layer(&src, &mut dst, w, h, &cfg);

        // (7, 7) wraps to diagonal neighbor of (0, 0)
        assert!(
            dst[(7 * w + 7) as usize] > 0.0,
            "wrapped diagonal neighbor (7,7) should get value"
        );
        // (0, 7) wraps to north neighbor of (0, 0)
        assert!(
            dst[(7 * w) as usize] > 0.0,
            "wrapped north neighbor (0,7) should get value"
        );
        // (7, 0) wraps to west neighbor of (0, 0)
        assert!(dst[7] > 0.0, "wrapped west neighbor (7,0) should get value");
    }

    // Smoke test for run_diffusion_phase
    #[test]
    fn run_diffusion_phase_smoke() {
        let config = WorldConfig {
            grid_width: 8,
            grid_height: 8,
            vent_count: 0,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);

        world.current_grid_mut()[0].pheromone = 100.0;
        world.current_grid_mut()[10].toxin = 50.0;
        world.current_grid_mut()[20].decay_energy = 30.0;

        run_diffusion_phase(&mut world, &config);

        assert!(world.current_grid()[0].pheromone < 100.0);
        assert!(world.current_grid()[10].toxin < 50.0);
        assert!(world.current_grid()[20].decay_energy < 30.0);
    }
}
