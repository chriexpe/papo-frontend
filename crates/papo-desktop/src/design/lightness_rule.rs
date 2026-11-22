//! `docs/design.md` §4's one rule for a colour that must stay readable: keep
//! its hue and chroma (lowered only where sRGB cannot hold them) and move
//! its lightness until its contrast floors hold. Highlights (a row's
//! background, held against the text on it) and role colours (a name, held
//! against the surfaces it sits on) are made by it.
//!
//! Owns the rule and its two uses; the tokens and the contrast measure are
//! `colour`'s.

use rune::{Color, oklch};

use super::Palette;
use super::colour::{Theme, contrast};

/// How far the lightness moves per step while it looks for the floors.
const LIGHTNESS_STEP: f32 = 0.002;
/// The contrast a role-coloured name keeps on `surface.sidebar` and
/// `surface.hover`.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the members pane and profile card (D5b) colour names by role"
    )
)]
const ROLE_NAME_FLOOR: f32 = 4.5;

/// The text a highlight must keep readable.
pub(super) struct Text {
    pub(super) primary: Color,
    pub(super) secondary: Color,
    pub(super) muted: Color,
    pub(super) link: Color,
}

/// A highlight at `hue_degrees` (§4, Highlights): from the theme's
/// background lightness and fixed chroma, the lightness moves away from the
/// text until `text.primary` keeps 13:1, `text.secondary` 7:1, and
/// `text.muted` and links 4.5:1.
pub(super) fn highlight(theme: Theme, hue_degrees: f32, text: &Text) -> Color {
    let (start, chroma) = theme.highlight_start();
    // Away from the text: darker under light text, lighter under dark text.
    let step = if theme.is_dark() {
        -LIGHTNESS_STEP
    } else {
        LIGHTNESS_STEP
    };
    reach_floor(start, chroma, hue_degrees, step, |background| {
        contrast(text.primary, background) >= 13.0
            && contrast(text.secondary, background) >= 7.0
            && contrast(text.muted, background) >= 4.5
            && contrast(text.link, background) >= 4.5
    })
}

/// A name in its highest coloured role's colour `role` (§4, Role colours):
/// the role's hue and chroma, its lightness moved toward the text until it
/// keeps 4.5:1 on `surface.sidebar` and `surface.hover`. A role colour that
/// already does keeps its lightness. Each step searches the gamut, so a
/// caller keeps the result per role and theme rather than asking in a view.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the members pane and profile card (D5b) colour names by role"
    )
)]
pub(crate) fn role_name(role: Color, palette: &Palette) -> Color {
    let [lightness, a, b, _] = role.to_oklab();
    let chroma = a.hypot(b);
    let hue_degrees = b.atan2(a).to_degrees().rem_euclid(360.0);
    let text_is_lighter =
        palette.text_primary.to_oklab()[0] > palette.surface_sidebar.to_oklab()[0];
    let step = if text_is_lighter {
        LIGHTNESS_STEP
    } else {
        -LIGHTNESS_STEP
    };
    reach_floor(lightness, chroma, hue_degrees, step, |name| {
        contrast(name, palette.surface_sidebar) >= ROLE_NAME_FLOOR
            && contrast(name, palette.surface_hover) >= ROLE_NAME_FLOOR
    })
}

/// The first colour at `hue_degrees` and (as much as sRGB holds of)
/// `chroma` that is `readable`, from lightness `start` on in `step`s;
/// the last one inside 0–1 when none is.
fn reach_floor(
    start: f32,
    chroma: f32,
    hue_degrees: f32,
    step: f32,
    readable: impl Fn(Color) -> bool,
) -> Color {
    let mut lightness = start;
    loop {
        let colour = oklch(
            lightness,
            in_gamut(lightness, chroma, hue_degrees),
            hue_degrees,
        );
        let next = lightness + step;
        if readable(colour) || !(0.0..=1.0).contains(&next) {
            return colour;
        }
        lightness = next;
    }
}

/// `chroma`, or the most of it sRGB can show at this lightness and hue.
pub(super) fn in_gamut(lightness: f32, chroma: f32, hue_degrees: f32) -> f32 {
    let mut chroma = chroma;
    // Rune clamps an unreachable colour; one that comes back unchanged fits.
    while chroma > 0.0 {
        let [l, a, b, _] = oklch(lightness, chroma, hue_degrees).to_oklab();
        let (sin, cos) = hue_degrees.to_radians().sin_cos();
        let fits = (l - lightness).abs() < 1e-3
            && (a - chroma * cos).abs() < 1e-3
            && (b - chroma * sin).abs() < 1e-3;
        if fits {
            return chroma;
        }
        chroma -= 0.001;
    }
    0.0
}
