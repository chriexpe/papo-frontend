//! Grain (`docs/design.md` §4, Background colour): a fine noise over the
//! whole window, above everything and taking no input, at the Grain
//! setting's strength.
//!
//! The mock lays grey noise over the window, blended *screen* on dark themes
//! and *multiply* on light ones. Screen with grey `n` at strength `a` is the
//! surface moved towards white by `a·n`, and multiply is it moved towards
//! black by `a·(1 − n)`: exactly white drawn at alpha `a·n`, or black at
//! alpha `a·(1 − n)`. So the grain is a tile of white (or black) whose alpha
//! is the noise, repeated over the window at opacity `a`, with no blend mode
//! needed. The tiles are made once, deterministically (a hash, not
//! randomness), and repeat without a seam.
//!
//! Owns the tiles and the overlay; the strength is `appearance`'s.

use std::sync::Arc;

use rune::{El, Image, ImageCache, el, memo, row};

/// A tile's side, in device pixels.
const TILE_PX: u32 = 256;
/// The finest octave's cell, in pixels: the grain's size (the mock's
/// `baseFrequency` of 0.85 per CSS pixel is about one cell a pixel).
const CELL_PX: u32 = 1;
/// The strength at Grain 1, on dark and light themes (the mock's 0.16 and
/// 0.22).
const STRENGTH_DARK: f32 = 0.16;
const STRENGTH_LIGHT: f32 = 0.22;

/// The two tiles, white noise for dark themes and black for light ones.
pub(crate) struct GrainTiles {
    white: Image,
    black: Image,
}

impl GrainTiles {
    /// The tile drawn on a dark or a light theme.
    pub(crate) fn pixels(&self, dark: bool) -> &rune::Pixels {
        if dark {
            self.white.pixels()
        } else {
            self.black.pixels()
        }
    }

    /// The grain's strength at `grain` (0–1) on a dark or a light theme:
    /// the overlay's opacity.
    pub(crate) fn strength(grain: f32, dark: bool) -> f32 {
        grain * if dark { STRENGTH_DARK } else { STRENGTH_LIGHT }
    }

    /// Makes both tiles; the cache decodes them off the UI thread.
    pub(crate) fn new(images: &ImageCache) -> GrainTiles {
        let noise = noise(TILE_PX);
        GrainTiles {
            white: images.rgba(TILE_PX, TILE_PX, tile(&noise, true)),
            black: images.rgba(TILE_PX, TILE_PX, tile(&noise, false)),
        }
    }
}

/// The grain over a window `size_pt` points across at `scale` device
/// pixels a point; none at Grain 0 or before the window has a size.
pub(crate) fn overlay(
    tiles: &GrainTiles,
    grain: f32,
    dark: bool,
    size_pt: Option<[f32; 2]>,
    scale: f32,
) -> Option<El> {
    let [width_pt, height_pt] = size_pt?;
    if grain <= 0.0 || scale <= 0.0 {
        return None;
    }
    let strength = GrainTiles::strength(grain, dark);
    #[expect(clippy::cast_precision_loss, reason = "a tile's side, 256")]
    let tile_pt = TILE_PX as f32 / scale;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window's size over a tile's: a few dozen"
    )]
    let (across, down) = (
        (width_pt / tile_pt).ceil() as u32,
        (height_pt / tile_pt).ceil() as u32,
    );
    let version = (dark, strength.to_bits(), tile_pt.to_bits(), across, down);
    #[expect(clippy::cast_precision_loss, reason = "a few dozen tiles")]
    let width_pt = tile_pt * across as f32;
    Some(memo("grain", version, || {
        let tile = tiles.pixels(dark);
        el().pin(Some(0.0), Some(0.0), None, None)
            .w(width_pt)
            .opacity(strength)
            .inert()
            .a11y_hidden()
            .child(
                row()
                    .w_full()
                    .flex_wrap()
                    .children((0..across * down).map(|_| el().size(tile_pt, tile_pt).pixels(tile))),
            )
    }))
}

/// Value noise in 0–1 over a `side`-pixel square that repeats seamlessly:
/// two octaves (the cell and twice it), each a hash of its lattice point,
/// smoothly interpolated, the finer weighted twice the coarser.
fn noise(side: u32) -> Vec<f32> {
    let mut values = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let fine = octave(x, y, CELL_PX, side, 0x9e37);
            let coarse = octave(x, y, CELL_PX * 2, side, 0x7f4a);
            values.push((fine * 2.0 + coarse) / 3.0);
        }
    }
    values
}

/// One octave at `(x, y)`: the lattice's hashed values around it, a
/// `cell`-pixel lattice wrapping at `side`, interpolated.
fn octave(x: u32, y: u32, cell: u32, side: u32, seed: u32) -> f32 {
    let lattice = (side / cell).max(1);
    let (cx, cy) = (x / cell, y / cell);
    #[expect(clippy::cast_precision_loss, reason = "a pixel inside a small cell")]
    let (fx, fy) = (
        (x % cell) as f32 / cell as f32,
        (y % cell) as f32 / cell as f32,
    );
    let at = |i: u32, j: u32| hash(i % lattice, j % lattice, seed);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (smooth(fx), smooth(fy));
    let top = at(cx, cy) + (at(cx + 1, cy) - at(cx, cy)) * sx;
    let bottom = at(cx, cy + 1) + (at(cx + 1, cy + 1) - at(cx, cy + 1)) * sx;
    top + (bottom - top) * sy
}

/// A value in 0–1 for a lattice point: an integer hash, the same on every
/// run.
fn hash(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    #[expect(clippy::cast_precision_loss, reason = "a fraction of the hash's range")]
    let value = (h >> 8) as f32 / (1u32 << 24) as f32;
    value
}

/// The tile's RGBA: white with the noise as its alpha, or black with its
/// complement (see the module's note).
fn tile(noise: &[f32], white: bool) -> Arc<[u8]> {
    let channel = if white { 255 } else { 0 };
    noise
        .iter()
        .flat_map(|&n| {
            let alpha = if white { n } else { 1.0 - n };
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an alpha in 0–1 scaled to a byte"
            )]
            let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            [channel, channel, channel, alpha]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TILE_PX, hash, noise, octave};

    #[test]
    #[expect(clippy::cast_precision_loss, reason = "a tile's pixel count")]
    fn the_noise_is_the_same_every_run_and_spread_over_its_range() {
        assert_eq!(hash(3, 4, 1), hash(3, 4, 1));
        let values = noise(TILE_PX);
        assert_eq!(values, noise(TILE_PX), "deterministic");
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.03, "centred: {mean}");
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        let spread = values.iter().map(|v| (v - mean).abs()).sum::<f32>() / values.len() as f32;
        assert!(spread > 0.1, "a visible grain: {spread}");
    }

    #[test]
    fn the_tile_repeats_without_a_seam() {
        // The lattice wraps at the tile's side, so the noise one tile over
        // is the noise here: the next tile carries straight on.
        for (x, y) in [(0, 0), (5, 9), (TILE_PX - 1, 3)] {
            for cell in [1, 2] {
                assert_eq!(
                    octave(x + TILE_PX, y, cell, TILE_PX, 7),
                    octave(x, y, cell, TILE_PX, 7)
                );
                assert_eq!(
                    octave(x, y + TILE_PX, cell, TILE_PX, 7),
                    octave(x, y, cell, TILE_PX, 7)
                );
            }
        }
    }
}
