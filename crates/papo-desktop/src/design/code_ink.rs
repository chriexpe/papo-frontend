//! Highlighted code's token colours (`docs/design.md` §9): each token kind
//! a fixed hue at chroma 0.12, its lightness moved until it reaches 4.5:1 on
//! `surface.code`.

use rune::{Color, oklch};

use super::background::Accent;
use super::colour::contrast;

/// A code token's OKLCH chroma (`docs/design.md` §9).
const CODE_TOKEN_CHROMA: f32 = 0.12;
/// The contrast a code token keeps on `surface.code`.
const CODE_TOKEN_FLOOR: f32 = 4.5;
/// How far a code token's lightness moves per step towards its floor.
const CODE_TOKEN_STEP: f32 = 0.01;

/// Highlighted code's token colours (`docs/design.md` §9). Comments take
/// `text.muted` and everything else `text.primary`, so only these five have
/// colours of their own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CodeInk {
    /// Keywords: the accent's hue.
    pub(crate) keyword: Color,
    pub(crate) string: Color,
    /// Numbers and constants.
    pub(crate) number: Color,
    pub(crate) type_name: Color,
    pub(crate) function: Color,
}

impl CodeInk {
    pub(super) fn new(dark: bool, surface_code: Color, accent: Accent) -> CodeInk {
        let token = |hue: f32| code_token(hue, dark, surface_code);
        // No token shares the keyword's hue: numbers move to 80° under the
        // Orange accent and functions to 256° under Teal.
        let number = if accent == Accent::Orange { 80.0 } else { 52.0 };
        let function = if accent == Accent::Teal { 256.0 } else { 196.0 };
        CodeInk {
            keyword: token(accent.hue_degrees()),
            string: token(150.0),
            number: token(number),
            type_name: token(300.0),
            function: token(function),
        }
    }
}

/// A code token of `hue`: lightness 0.78 on dark themes and 0.48 on light
/// ones, moved away from `surface_code` until it reaches 4.5:1 on it.
fn code_token(hue: f32, dark: bool, surface_code: Color) -> Color {
    let (mut lightness, step) = if dark {
        (0.78, CODE_TOKEN_STEP)
    } else {
        (0.48, -CODE_TOKEN_STEP)
    };
    let mut colour = oklch(lightness, CODE_TOKEN_CHROMA, hue);
    while contrast(colour, surface_code) < CODE_TOKEN_FLOOR && (0.0..=1.0).contains(&lightness) {
        lightness += step;
        colour = oklch(lightness, CODE_TOKEN_CHROMA, hue);
    }
    colour
}
