//! Renderer: turns a `WorldSnapshot` into an RGBA pixel buffer.
#![allow(dead_code)]
//!
//! The renderer owns no simulation state — it is a pure projection from
//! snapshot → framebuffer, so the main thread can render the latest
//! published snapshot without touching the sim thread.

pub mod color;

use crate::render::color::{ColorMode, cell_to_rgba};
use crate::sim::world::WorldSnapshot;

/// Fixed-size RGBA framebuffer. Stateless today, but holds width/height so
/// that future tile overlays can be pre-computed once and reused.
pub struct Renderer {
    pub width: u32,
    pub height: u32,
    /// What cell pixels encode. Switchable at runtime for debugging.
    pub mode: ColorMode,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mode: ColorMode::default(),
        }
    }

    /// Switch what cell pixels encode (strategy, phase, energy, genetic).
    pub fn set_mode(&mut self, mode: ColorMode) {
        self.mode = mode;
    }

    /// Produce an `RGBA8` pixel buffer (`width * height * 4` bytes) from a
    /// snapshot. Cells are drawn over a background fill.
    pub fn render(&self, snapshot: &WorldSnapshot) -> Vec<u8> {
        let total = (self.width * self.height) as usize;
        let mut buf = vec![0u8; total * 4];

        self.fill_background(snapshot, &mut buf);

        for cell in &snapshot.cells {
            let idx = (cell.y as u32 * self.width + cell.x as u32) as usize * 4;
            if idx + 4 <= buf.len() {
                buf[idx..idx + 4].copy_from_slice(&cell_to_rgba(cell, self.mode));
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
    use crate::sim::world::{CellView, SimStats, Strategy};

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

    fn view_at(x: u16, y: u16) -> CellView {
        CellView {
            x,
            y,
            genome: GenomeHash(0x1234_5678),
            strategy: Strategy::Photosynthesis,
            specialization: 0.7,
            energy_fraction: 0.6,
            age_fraction: 0.2,
            active_phase: 1,
        }
    }

    #[test]
    fn cell_pixel_matches_cell_color() {
        let r = Renderer::new(8, 8);
        let mut snap = empty_snapshot(8, 8);
        let cell = view_at(3, 4);
        snap.cells.push(cell);

        let buf = r.render(&snap);
        let idx = (4 * 8 + 3) * 4;
        assert_eq!(&buf[idx..idx + 4], &cell_to_rgba(&cell, r.mode));
    }

    #[test]
    fn switching_mode_changes_the_pixel() {
        let mut r = Renderer::new(8, 8);
        let mut snap = empty_snapshot(8, 8);
        snap.cells.push(view_at(3, 4));
        let idx = (4 * 8 + 3) * 4;

        let genetic = r.render(&snap)[idx..idx + 4].to_vec();
        r.set_mode(ColorMode::Phase);
        let phase = r.render(&snap)[idx..idx + 4].to_vec();
        assert_ne!(genetic, phase, "mode switch must change what is drawn");
    }

    #[test]
    fn cells_outside_bounds_do_not_panic() {
        // Defensive: snapshot coords should always be in-range, but the
        // renderer must not panic if stale data arrives.
        let r = Renderer::new(4, 4);
        let mut snap = empty_snapshot(4, 4);
        snap.cells.push(view_at(10, 10));
        let _ = r.render(&snap);
    }
}
