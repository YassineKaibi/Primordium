// @veridikt
// kind: module
// name: Diffusion
// purpose: "Generic toroidal field diffusion + decay, run once per tick over pheromone, toxin, and temperature layers, plus decay-matter fade"
// owner: "primordium-maintainers"
// because: "One parameterized diffuse function serves all three layers (each with its own decay/spread/neighborhood) so signal, pollution, and heat share identical, tested spreading math"

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
    /// Results smaller than this in magnitude are written as exactly zero.
    pub flush: f32,
}

// ── Core diffusion function ────────────────────────────────────────

/// Diffuse a scalar field from `src` into `dst` on a toroidal grid.
///
/// Formula per cell:
///   new = value * (1 - decay_rate - spread_rate)
///       + sum(neighbor_values) * (spread_rate / neighbor_count)
///
/// `src` and `dst` must both have length `width * height`.

// @veridikt
// purpose: "Spread one scalar field one step on the toroidal grid: each tile keeps a retained fraction and gains an equal share from its neighbors"
// because: "retain = 1 - decay - spread keeps the field conservative apart from the explicit decay term, which is what lets pheromone/toxin form smooth gradients cells can climb"
// assumes: "src and dst are both width*height long; src is not aliased by dst (caller passes a clone)"
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

    // Interior tiles need no wrapping, so their neighbours are plain index
    // offsets. Only the border pays for rem_euclid. Same summation order as
    // `offsets`, so the result is bit-identical to the wrapping path.
    let stride = width as usize;
    let interior: Vec<isize> = offsets
        .iter()
        .map(|&(dx, dy)| dy as isize * stride as isize + dx as isize)
        .collect();

    // A row whose own values and both neighbouring rows' values are all zero
    // diffuses to exactly zero, so it is written as zero and skipped. Most of
    // the grid is empty most of the time, and every processed tile below is
    // computed exactly as before.
    let row_live: Vec<bool> = (0..height as usize)
        .map(|y| src[y * stride..(y + 1) * stride].iter().any(|&v| v != 0.0))
        .collect();

    for y in 0..height {
        let yu = y as usize;
        let above = row_live[(yu + height as usize - 1) % height as usize];
        let below = row_live[(yu + 1) % height as usize];
        if !(row_live[yu] || above || below) {
            dst[yu * stride..(yu + 1) * stride].fill(0.0);
            continue;
        }
        let on_border_row = y == 0 || y + 1 == height;
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let mut neighbor_sum = 0.0;

            if on_border_row || x == 0 || x + 1 == width {
                for &(dx, dy) in offsets {
                    let nx = (x as i32 + dx).rem_euclid(w) as u32;
                    let ny = (y as i32 + dy).rem_euclid(h) as u32;
                    neighbor_sum += src[(ny * width + nx) as usize];
                }
            } else {
                for &off in &interior {
                    neighbor_sum += src[idx.wrapping_add_signed(off)];
                }
            }

            let value = src[idx] * retain + neighbor_sum * spread_per_neighbor;
            dst[idx] = if value.abs() < config.flush {
                0.0
            } else {
                value
            };
        }
    }
}

// ── Decay matter fade ──────────────────────────────────────────────

/// Fade the decay matter on every tile.
///
/// Each tile fades at its own `decay_fade`, written when a corpse landed
/// there from that cell's `decay_rate` gene (`energy::corpse_decay_fade`).
/// `default_rate` covers tiles that hold decay from no particular corpse.

// @veridikt
// purpose: "Fade each tile's decay matter at the rate the corpse that made it set, falling back to the world rate"
// because: "spec.md gene 30 makes corpse persistence a trait of the dead cell, so the rate has to travel with the deposit rather than being one global constant"
pub fn fade_decay(tiles: &mut [Tile], default_rate: f32) {
    for tile in tiles.iter_mut() {
        if tile.decay_energy <= 0.0 {
            continue;
        }
        let rate = if tile.decay_fade > 0.0 {
            tile.decay_fade
        } else {
            default_rate
        };
        tile.decay_energy *= 1.0 - rate;
        // Multiplicative fade never reaches zero on its own, so every tile
        // that ever held decay — which, with `initial_decay_matter`, is every
        // tile — kept a trace forever. That trace is worth nothing (0.01 of
        // decay scavenges to 0.009 energy against an upkeep near 1), but it
        // stopped `recompute_sunlight` treating the tile as plain water.
        if tile.decay_energy < DECAY_FLUSH {
            tile.decay_energy = 0.0;
        }
    }
}

/// Decay matter below this is cleared to exactly zero. A numerical floor,
/// not a balance knob: at 0.01 a scavenger would earn 0.009 per tick.
pub const DECAY_FLUSH: f32 = 0.01;

/// Pheromone and toxin below this are cleared to exactly zero. Emission is
/// on the order of 0.1-1.0 per cell per tick, so this is four orders of
/// magnitude under anything a cell could sense, and it is what lets an empty
/// region become *exactly* empty and be skipped.
pub const FIELD_FLUSH: f32 = 1e-4;

/// Diffuse one `f32` field of the tile grid through the shared scratch
/// buffers.
///
/// A layer that is zero everywhere diffuses to zero everywhere, so it is
/// skipped outright — the usual state of toxin, which only forms where at
/// least `toxin_generation_threshold` cells have died close together. The
/// gather already reads every value, so checking costs nothing extra.

// @veridikt
// purpose: "Gather one tile field into the scratch buffer, diffuse it, and write it back — or skip all three when the field is empty"
// because: "Diffusion was a fixed per-tick cost over every tile regardless of population; an empty field (toxin, most of the time) has nothing to spread"
fn diffuse_field(
    world: &mut World,
    config: &DiffusionConfig,
    read: impl Fn(&Tile) -> f32,
    write: impl Fn(&mut Tile, f32),
) {
    let (width, height) = (world.width, world.height);
    // Take the scratch buffers out so the grid can be borrowed alongside.
    let mut src = std::mem::take(&mut world.diffusion_a);
    let mut dst = std::mem::take(&mut world.diffusion_b);

    let mut any = false;
    for (slot, tile) in src.iter_mut().zip(world.current_grid().iter()) {
        let v = read(tile);
        any |= v != 0.0;
        *slot = v;
    }
    if any {
        diffuse_layer(&src, &mut dst, width, height, config);
        for (tile, &v) in world.current_grid_mut().iter_mut().zip(dst.iter()) {
            write(tile, v);
        }
    }

    world.diffusion_a = src;
    world.diffusion_b = dst;
}

// ── Orchestration ──────────────────────────────────────────────────

/// Run all three diffusion passes (pheromone, toxin, temperature) plus
/// decay fade, reusing the two float buffers from World.

// @veridikt
// purpose: "Tick phase 1 — diffuse pheromone (Moore), toxin (Von Neumann) and temperature (Moore) on the current grid, then fade decay matter"
// because: "Passes reuse World.diffusion_a/diffusion_b scratch buffers instead of allocating per tick; pheromone uses Moore (8-way) while toxin uses Von Neumann (4-way) to give them visibly different spread shapes"
pub fn run_diffusion_phase(world: &mut World, config: &WorldConfig) {
    let width = world.width;
    let height = world.height;
    let total = (width * height) as usize;

    // ── Pheromone (Moore) ──────────────────────────────────────
    let pheromone_config = DiffusionConfig {
        decay_rate: config.pheromone_decay,
        spread_rate: config.pheromone_diffusion,
        neighborhood: Neighborhood::Moore,
        flush: FIELD_FLUSH,
    };

    diffuse_field(
        world,
        &pheromone_config,
        |t| t.pheromone,
        |t, v| t.pheromone = v,
    );

    // ── Toxin (Von Neumann) ────────────────────────────────────
    let toxin_config = DiffusionConfig {
        decay_rate: config.toxin_decay,
        spread_rate: config.toxin_diffusion,
        neighborhood: Neighborhood::VonNeumann,
        flush: FIELD_FLUSH,
    };

    diffuse_field(world, &toxin_config, |t| t.toxin, |t, v| t.toxin = v);

    // ── Temperature (Moore, u8 -> f32 -> u8) ───────────────────
    // A tile's update moves it by at most `255 * (decay + spread)`: the decay
    // term, plus the spread term pulling it toward a neighbour average that
    // can differ from it by at most 255. Below half a degree, rounding sends
    // every tile straight back to where it was, so the whole pass is a
    // provable no-op and is skipped. At the default rates the bound is 0.28.
    let max_step = 255.0 * (config.temperature_decay + config.temperature_diffusion);
    if max_step < 0.5 {
        fade_decay(world.current_grid_mut(), config.decay_rate);
        return;
    }
    let temp_config = DiffusionConfig {
        decay_rate: config.temperature_decay,
        spread_rate: config.temperature_diffusion,
        neighborhood: Neighborhood::Moore,
        // Temperature rounds back to u8 below, so no separate floor is needed.
        flush: 0.0,
    };

    for i in 0..total {
        world.diffusion_a[i] = world.current_grid()[i].temperature as f32;
    }
    diffuse_layer(
        &world.diffusion_a,
        &mut world.diffusion_b,
        width,
        height,
        &temp_config,
    );
    // Round, never truncate. `as u8` floors, so a tile at 128 whose update
    // came to 127.99 became 127 — one full degree lost every tick, about
    // 10 000x the configured `temperature_decay`. The map `spec.md` calls
    // static ("Perlin noise at init, rarely changes") averaged 219.8 at tick 0
    // and **0.0 by tick 300**, and every cell after that paid its whole
    // `temperature_preference` as mismatch cost against a map of zeros.
    for i in 0..total {
        world.current_grid_mut()[i].temperature =
            world.diffusion_b[i].round().clamp(0.0, 255.0) as u8;
    }

    // ── Decay fade ─────────────────────────────────────────────
    fade_decay(world.current_grid_mut(), config.decay_rate);
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty field must stay exactly empty and cost nothing, and a live
    /// one must spread exactly as before. Rows far from any value are skipped
    /// rather than computed, which is only safe if skipping and computing
    /// give the same answer there.
    #[test]
    fn skipping_empty_rows_changes_nothing_that_was_computed() {
        let (w, h) = (16u32, 16u32);
        let mut src = make_grid(w, h);
        src[(8 * w + 8) as usize] = 1.0;
        let config = moore_config(0.05, 0.15);

        let mut sparse = make_grid(w, h);
        diffuse_layer(&src, &mut sparse, w, h, &config);

        // Only rows 7, 8 and 9 can be non-zero after one step.
        for y in 0..h {
            let row = &sparse[(y * w) as usize..((y + 1) * w) as usize];
            if !(7..=9).contains(&y) {
                assert!(row.iter().all(|&v| v == 0.0), "row {y} should be empty");
            }
        }
        // And the centre kept exactly its retained share.
        assert_eq!(sparse[(8 * w + 8) as usize], 1.0 * (1.0 - 0.05 - 0.15));
    }

    /// Multiplicative fade never reaches zero, so without a floor every tile
    /// that ever held decay kept a trace forever and could never take the
    /// plain-water path in `recompute_sunlight`.
    #[test]
    fn decay_fades_all_the_way_to_zero() {
        let mut tiles = vec![Tile::EMPTY; 1];
        tiles[0].decay_energy = 8.0;
        for _ in 0..1000 {
            fade_decay(&mut tiles, 0.02);
        }
        assert_eq!(tiles[0].decay_energy, 0.0);
    }

    /// `spec.md`: temperature is "static (Perlin noise at init), rarely
    /// changes" and diffuses "near-zero, very slow". Truncating the `u8` on
    /// write-back lost a whole degree per tick, so the map averaged 219.8 at
    /// tick 0 and 0.0 by tick 300.
    #[test]
    fn the_temperature_map_stays_put_under_its_configured_rates() {
        let config = WorldConfig {
            grid_width: 64,
            grid_height: 64,
            ..WorldConfig::default()
        };
        let mut world = World::new(&config);
        let mean = |w: &World| {
            w.current_grid()
                .iter()
                .map(|t| t.temperature as f64)
                .sum::<f64>()
                / (64.0 * 64.0)
        };
        let before = mean(&world);
        for _ in 0..300 {
            run_diffusion_phase(&mut world, &config);
        }
        let after = mean(&world);
        assert!(
            (before - after).abs() < 1.0,
            "mean temperature drifted from {before:.1} to {after:.1} in 300 ticks"
        );
    }

    use crate::config::WorldConfig;

    fn make_grid(width: u32, height: u32) -> Vec<f32> {
        vec![0.0; (width * height) as usize]
    }

    fn moore_config(decay: f32, spread: f32) -> DiffusionConfig {
        DiffusionConfig {
            decay_rate: decay,
            spread_rate: spread,
            neighborhood: Neighborhood::Moore,
            // No floor: these tests check the exact spreading arithmetic.
            flush: 0.0,
        }
    }

    fn vn_config(decay: f32, spread: f32) -> DiffusionConfig {
        DiffusionConfig {
            decay_rate: decay,
            spread_rate: spread,
            neighborhood: Neighborhood::VonNeumann,
            flush: 0.0,
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
