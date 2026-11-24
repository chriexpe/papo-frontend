//! Highlights on glass (`docs/design.md` §5, Glass; Christian, 2026-11-24):
//! a row highlighted on a floating surface is a tint, not an opaque block,
//! so what is behind the glass shows through it. The tint is the least
//! opaque colour that, drawn over the surface's own colour, gives exactly
//! the highlight's token: on a solid surface it is the token, so the
//! contrast floors that hold for the token hold for it.
//!
//! Owns that one computation; which surfaces are glass is `Look`'s.

use rune::Color;
use rune::color::{srgb_decode, srgb_encode};

/// The least opaque colour that, composited over `base`, gives `target`.
/// Rune blends in sRGB (gamma) space (`App::srgb_blend`, on by default), so
/// it is solved there: each channel needs an opacity of at least its
/// distance to `target` over its room to move that way, and the tint takes
/// the largest, which puts one channel at the end of its range. A `target`
/// equal to `base` gives a transparent tint.
pub(crate) fn tint_over(base: Color, target: Color) -> Color {
    let base_s = [base.r, base.g, base.b].map(srgb_encode);
    let target_s = [target.r, target.g, target.b].map(srgb_encode);
    let alpha = base_s
        .iter()
        .zip(target_s)
        .map(|(&from, to)| {
            if to > from {
                (to - from) / (1.0 - from).max(f32::EPSILON)
            } else {
                (from - to) / from.max(f32::EPSILON)
            }
        })
        .fold(0.0_f32, f32::max)
        .clamp(0.0, 1.0);
    if alpha <= f32::EPSILON {
        return target.alpha(0.0);
    }
    let [r, g, b] = [0, 1, 2].map(|channel| {
        let from = base_s[channel];
        srgb_decode((from + (target_s[channel] - from) / alpha).clamp(0.0, 1.0))
    });
    Color { r, g, b, a: alpha }
}

#[cfg(test)]
mod tests {
    use rune::Color;
    use rune::color::srgb_encode;

    use super::tint_over;

    /// `tint` drawn over `base`, as Rune blends: in sRGB.
    fn over(tint: Color, base: Color) -> [f32; 3] {
        let channel = |t: f32, b: f32| srgb_encode(t) * tint.a + srgb_encode(b) * (1.0 - tint.a);
        [
            channel(tint.r, base.r),
            channel(tint.g, base.g),
            channel(tint.b, base.b),
        ]
    }

    #[test]
    fn the_tint_over_its_surface_is_the_highlight() {
        let surfaces = [
            rune::oklch(0.27, 0.01, 255.0),
            rune::oklch(0.175, 0.0, 0.0),
            rune::oklch(1.0, 0.0, 0.0),
            rune::oklch(0.95, 0.01, 90.0),
        ];
        let highlights = [
            rune::oklch(0.35, 0.05, 256.0),
            rune::oklch(0.2, 0.02, 200.0),
            rune::oklch(0.9, 0.04, 30.0),
        ];
        for base in surfaces {
            for target in highlights {
                let tint = tint_over(base, target);
                let drawn = over(tint, base);
                let wanted = [target.r, target.g, target.b].map(srgb_encode);
                for (got, want) in drawn.iter().zip(wanted) {
                    assert!((got - want).abs() < 1e-4, "{got} for {want}");
                }
                assert!(tint.a <= 1.0);
            }
        }
    }

    #[test]
    fn the_tint_is_as_clear_as_it_can_be() {
        let base = rune::oklch(0.27, 0.01, 255.0);
        let tint = tint_over(base, rune::oklch(0.33, 0.01, 255.0));
        assert!(tint.a < 0.2, "a small step is a faint tint: {}", tint.a);
        // One channel at the end of its range: a clearer tint could not
        // reach the highlight.
        let ends = [tint.r, tint.g, tint.b]
            .iter()
            .any(|channel| *channel > 0.999 || *channel < 0.001);
        assert!(ends, "{tint:?}");
    }

    #[test]
    fn a_highlight_equal_to_its_surface_draws_nothing() {
        let base = rune::oklch(0.27, 0.01, 255.0);
        assert!(tint_over(base, base).a.abs() < f32::EPSILON);
    }
}
