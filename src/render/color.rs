//! Genome-to-color mapping.
#![allow(dead_code)]
//!
//! Color is a continuous function of what a cell *is*: its acquisition
//! strategy sets the hue band, its genome varies the hue within that band,
//! and its energy sets the brightness. Two cells one mutation apart land
//! next to each other; a cell from another lineage does not.

use crate::sim::world::{CellView, Strategy};

/// What a rendered pixel encodes.
///
/// April's log asked for exactly this split: a strict debug view that shows
/// strategy only ("good for debugging but too restrictive"), and a normal
/// view that is a hybrid of strategy and genetic identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Strategy hue + per-genome variation + energy brightness.
    #[default]
    Genetic,
    /// Flat color per strategy class. Reads population structure at a glance.
    Strategy,
    /// Flat color per active phase slot. Answers "did phases evolve?".
    Phase,
    /// Heat ramp on energy as a share of the cell's own cap.
    Energy,
}

/// Hue band centre for each strategy, in degrees.
fn strategy_hue(strategy: Strategy) -> f32 {
    match strategy {
        Strategy::Photosynthesis => 110.0, // green
        Strategy::Thermosynthesis => 20.0, // ember
        Strategy::Scavenging => 45.0,      // ochre
        Strategy::Predation => 300.0,      // magenta
        Strategy::None => 210.0,           // slate
    }
}

/// Half-width of the hue band a lineage can drift within, in degrees.
const HUE_SPREAD: f32 = 22.0;

/// Convert a cell into an opaque RGBA pixel under the given mode.
pub fn cell_to_rgba(cell: &CellView, mode: ColorMode) -> [u8; 4] {
    let (h, s, v) = match mode {
        ColorMode::Genetic => {
            // Hue drifts within the strategy's band, driven by the genome
            // hash; saturation shows how specialized the cell is; value
            // shows how well fed it is.
            let offset = (hash_unit(cell.genome.0) * 2.0 - 1.0) * HUE_SPREAD;
            let hue = (strategy_hue(cell.strategy) + offset).rem_euclid(360.0);
            let sat = 0.35 + 0.6 * cell.specialization;
            let val = 0.45 + 0.55 * cell.energy_fraction;
            (hue, sat, val)
        }
        ColorMode::Strategy => (strategy_hue(cell.strategy), 0.85, 0.9),
        ColorMode::Phase => match cell.active_phase {
            0 => (0.0, 0.0, 0.55), // grey: no phase active
            1 => (190.0, 0.85, 0.95),
            2 => (55.0, 0.9, 0.95),
            3 => (325.0, 0.85, 0.95),
            _ => (0.0, 0.0, 1.0),
        },
        ColorMode::Energy => {
            // Dark red (starving) through yellow to white (full).
            let f = cell.energy_fraction;
            (f * 60.0, 1.0 - f * 0.7, 0.35 + 0.65 * f)
        }
    };
    let (r, g, b) = hsv_to_rgb(h, s, v);
    [r, g, b, 255]
}

/// Spread the hash bits into [0, 1). Uses all 32 bits so that a one-byte
/// genome change moves the value, but only within the strategy's band.
fn hash_unit(bits: u32) -> f32 {
    let mut x = bits;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    (x >> 8) as f32 / 16_777_216.0
}

/// HSV → RGB. `h` in degrees [0, 360), `s` and `v` in [0, 1].
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::genome::GenomeHash;

    fn view(strategy: Strategy, hash: u32) -> CellView {
        CellView {
            x: 0,
            y: 0,
            genome: GenomeHash(hash),
            strategy,
            specialization: 0.8,
            energy_fraction: 0.8,
            age_fraction: 0.1,
            active_phase: 0,
        }
    }

    /// Hue of an RGB triple, in degrees.
    fn hue_of(rgba: [u8; 4]) -> f32 {
        let (r, g, b) = (
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        if d < 1e-6 {
            return 0.0;
        }
        let h = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        h.rem_euclid(360.0)
    }

    fn hue_distance(a: f32, b: f32) -> f32 {
        let d = (a - b).abs() % 360.0;
        d.min(360.0 - d)
    }

    #[test]
    fn same_strategy_stays_in_one_hue_band() {
        // The old hash coloring moved hue ~88 degrees for a one-byte change,
        // indistinguishable from an unrelated genome (~90 degrees). Cells of
        // one strategy must now read as one family.
        let mut worst = 0.0_f32;
        for hash in [0u32, 1, 0xdead_beef, 0x1234_5678, 0xffff_ffff, 7, 99_991] {
            let h = hue_of(cell_to_rgba(
                &view(Strategy::Photosynthesis, hash),
                ColorMode::Genetic,
            ));
            worst = worst.max(hue_distance(h, strategy_hue(Strategy::Photosynthesis)));
        }
        assert!(
            worst <= HUE_SPREAD + 1.0,
            "hue drifted {worst} degrees, band is {HUE_SPREAD}"
        );
    }

    #[test]
    fn different_strategies_are_far_apart() {
        let photo = hue_of(cell_to_rgba(
            &view(Strategy::Photosynthesis, 42),
            ColorMode::Genetic,
        ));
        let hunter = hue_of(cell_to_rgba(
            &view(Strategy::Predation, 42),
            ColorMode::Genetic,
        ));
        assert!(
            hue_distance(photo, hunter) > 3.0 * HUE_SPREAD,
            "strategies must not be confusable: {photo} vs {hunter}"
        );
    }

    #[test]
    fn energy_shows_as_brightness() {
        let mut starving = view(Strategy::Photosynthesis, 42);
        starving.energy_fraction = 0.0;
        let mut full = view(Strategy::Photosynthesis, 42);
        full.energy_fraction = 1.0;

        let dim = *cell_to_rgba(&starving, ColorMode::Genetic)[..3]
            .iter()
            .max()
            .unwrap();
        let bright = *cell_to_rgba(&full, ColorMode::Genetic)[..3]
            .iter()
            .max()
            .unwrap();
        assert!(
            bright > dim,
            "full cell {bright} should outshine starving {dim}"
        );
    }

    #[test]
    fn phase_mode_separates_every_slot() {
        let mut seen = Vec::new();
        for phase in 0..4u8 {
            let mut v = view(Strategy::Photosynthesis, 42);
            v.active_phase = phase;
            seen.push(cell_to_rgba(&v, ColorMode::Phase));
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                assert_ne!(seen[i], seen[j], "phases {i} and {j} look identical");
            }
        }
    }

    #[test]
    fn strategy_mode_ignores_genome() {
        let a = cell_to_rgba(&view(Strategy::Scavenging, 1), ColorMode::Strategy);
        let b = cell_to_rgba(
            &view(Strategy::Scavenging, 0xffff_ffff),
            ColorMode::Strategy,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn alpha_is_opaque() {
        for mode in [
            ColorMode::Genetic,
            ColorMode::Strategy,
            ColorMode::Phase,
            ColorMode::Energy,
        ] {
            assert_eq!(cell_to_rgba(&view(Strategy::None, 3), mode)[3], 255);
        }
    }

    #[test]
    fn hsv_round_trip_primaries() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
    }
}
