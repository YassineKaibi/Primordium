//! Renderer: turns a `WorldSnapshot` into an RGBA pixel buffer.
#![allow(dead_code)]
//!
//! The renderer owns no simulation state — it is a pure projection from
//! snapshot → framebuffer, so the main thread can render the latest
//! published snapshot without touching the sim thread.

pub mod color;

use crate::render::color::genome_hash_to_rgba;
use crate::sim::world::WorldSnapshot;

/// Fixed-size RGBA framebuffer. Stateless today, but holds width/height so
/// that future tile overlays can be pre-computed once and reused.
pub struct Renderer {
    pub width: u32,
    pub height: u32,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Produce an `RGBA8` pixel buffer (`width * height * 4` bytes) from a
    /// snapshot. Cells are drawn over a background fill.
    pub fn render(&self, snapshot: &WorldSnapshot) -> Vec<u8> {
        let total = (self.width * self.height) as usize;
        let mut buf = vec![0u8; total * 4];

        self.fill_background(snapshot, &mut buf);

        for &(x, y, hash) in &snapshot.cells {
            let idx = (y as u32 * self.width + x as u32) as usize * 4;
            if idx + 4 <= buf.len() {
                buf[idx..idx + 4].copy_from_slice(&genome_hash_to_rgba(hash));
            }
        }

        buf
    }

    /// Paint the background tiles (everything that isn't a live cell).
    ///
    /// Each tile blends decay (brown), pheromone (pink), and toxin (purple)
    /// via soft-saturation. Cell pixels are written on top, so this affects
    /// only empty tiles visually.
    fn fill_background(&self, snapshot: &WorldSnapshot, buf: &mut [u8]) {
        let total = (self.width * self.height) as usize;

        for idx in 0..total {
            let d = snapshot.decay_map[idx];
            let p = snapshot.pheromone_map[idx];
            let t = snapshot.toxin_map[idx];

            let decay_color = d / (d + 5.0);
            let pheromone_color = p / (p + 2.0);
            let toxin_color = t / (t + 1.5);

            let r =
                (decay_color * 60.0 + pheromone_color * 70.0 + toxin_color * 40.0).min(80.0) as u8;
            let g =
                (decay_color * 40.0 + pheromone_color * 20.0 + toxin_color * 10.0).min(80.0) as u8;
            let b =
                (decay_color * 20.0 + pheromone_color * 50.0 + toxin_color * 60.0).min(80.0) as u8;

            buf[idx * 4..idx * 4 + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::genome::GenomeHash;
    use crate::sim::world::SimStats;

    fn empty_snapshot(w: u32, h: u32) -> WorldSnapshot {
        let total = (w * h) as usize;
        WorldSnapshot {
            tick: 0,
            cells: vec![],
            decay_map: vec![0.0; total],
            pheromone_map: vec![0.0; total],
            toxin_map: vec![0.0; total],
            stats: SimStats::default(),
        }
    }

    #[test]
    fn output_length_matches_resolution() {
        let r = Renderer::new(16, 9);
        let snap = empty_snapshot(16, 9);
        assert_eq!(r.render(&snap).len(), 16 * 9 * 4);
    }

    #[test]
    fn cell_pixel_matches_genome_color() {
        let r = Renderer::new(8, 8);
        let mut snap = empty_snapshot(8, 8);
        let hash = GenomeHash(0x1234_5678);
        snap.cells.push((3, 4, hash));

        let buf = r.render(&snap);
        let idx = (4 * 8 + 3) * 4;
        assert_eq!(&buf[idx..idx + 4], &genome_hash_to_rgba(hash));
    }

    #[test]
    fn cells_outside_bounds_do_not_panic() {
        // Defensive: snapshot coords should always be in-range, but the
        // renderer must not panic if stale data arrives.
        let r = Renderer::new(4, 4);
        let mut snap = empty_snapshot(4, 4);
        snap.cells.push((10, 10, GenomeHash(0)));
        let _ = r.render(&snap);
    }
}
