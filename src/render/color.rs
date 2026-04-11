//! Genome-to-color mapping.
#![allow(dead_code)]
//!
//! Maps a `GenomeHash` to an RGBA pixel by splitting the hash bits into
//! HSV channels. Lineage drift is visible because the underlying genome
//! changes gradually, not because the hash itself is continuous.

use crate::sim::genome::GenomeHash;

/// Convert a genome hash into an opaque RGBA pixel.
///
/// Bit layout of the 32-bit hash:
/// - upper 16 bits → hue  (full 0..360 range, most entropy)
/// - next 8 bits   → saturation (clamped to 0.55..1.0 so cells stay vivid)
/// - low 8 bits    → value      (clamped to 0.65..1.0 so cells stay bright)
pub fn genome_hash_to_rgba(hash: GenomeHash) -> [u8; 4] {
    let bits = hash.0;
    let hue = (bits >> 16) as f32 / 65_535.0 * 360.0;
    let sat = 0.55 + ((bits >> 8) & 0xff) as f32 / 255.0 * 0.45;
    let val = 0.65 + (bits & 0xff) as f32 / 255.0 * 0.35;
    let (r, g, b) = hsv_to_rgb(hue, sat, val);
    [r, g, b, 255]
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

    #[test]
    fn same_hash_produces_same_color() {
        let h = GenomeHash(0xdead_beef);
        assert_eq!(genome_hash_to_rgba(h), genome_hash_to_rgba(h));
    }

    #[test]
    fn alpha_is_opaque() {
        for bits in [0, 0x1234_5678, 0xffff_ffff, 0x0000_ffff] {
            assert_eq!(genome_hash_to_rgba(GenomeHash(bits))[3], 255);
        }
    }

    #[test]
    fn nearby_hashes_have_similar_hues() {
        // Two hashes differing only in the low bits share the same upper-16
        // bit field, so their hues must be identical.
        let a = genome_hash_to_rgba(GenomeHash(0xabcd_0000));
        let b = genome_hash_to_rgba(GenomeHash(0xabcd_00ff));
        // Reverse HSV won't match exactly (s/v differ), but R/G/B ordering
        // should be preserved because hue is the dominant channel.
        let max_a = *a[..3].iter().max().unwrap();
        let max_b = *b[..3].iter().max().unwrap();
        let min_a = *a[..3].iter().min().unwrap();
        let min_b = *b[..3].iter().min().unwrap();
        assert_eq!(
            a[..3].iter().position(|&c| c == max_a),
            b[..3].iter().position(|&c| c == max_b),
            "dominant channel must match"
        );
        assert_eq!(
            a[..3].iter().position(|&c| c == min_a),
            b[..3].iter().position(|&c| c == min_b),
            "recessive channel must match"
        );
    }

    #[test]
    fn minimum_brightness_floor() {
        // Value is floored at 0.65, so no component should be near-black.
        let rgba = genome_hash_to_rgba(GenomeHash(0));
        let max = *rgba[..3].iter().max().unwrap();
        assert!(max >= (0.65 * 255.0) as u8);
    }

    #[test]
    fn hsv_round_trip_primaries() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
    }
}
