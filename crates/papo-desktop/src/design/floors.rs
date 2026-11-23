//! Fitting a colour to its contrast floors (`docs/design.md` §4). The
//! design's values are targets; with any accent and background colour, a
//! colour that would break a floor gives way as little as it must, never
//! more:
//!
//! - a background that carries meaning (a highlight, the selected row, a
//!   tile), and a colour drawn on surfaces (a link, a role-coloured name),
//!   keep their hue and chroma and move their lightness until the floors
//!   hold;
//! - a tinted neutral surface keeps its lightness, so the steps between
//!   surfaces stay as authored, and loses tint until the text on it holds.
//!
//! Owns the rules and their uses; the tokens and the contrast measure are
//! `colour`'s.

use rune::{Color, oklch};

use super::Palette;
use super::colour::{Theme, contrast};

/// How far the lightness moves per step while it looks for the floors.
const LIGHTNESS_STEP: f32 = 0.002;
/// How much chroma a tinted surface gives up per step.
const CHROMA_STEP: f32 = 0.001;
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

impl Text {
    /// Whether every text floor holds on `background`: `text.primary`
    /// 13:1, `text.secondary` 7:1, `text.muted` 4.5:1. Links are fitted to
    /// the surfaces afterwards (`link`).
    pub(super) fn reads_on(&self, background: Color) -> bool {
        contrast(self.primary, background) >= 13.0
            && contrast(self.secondary, background) >= 7.0
            && contrast(self.muted, background) >= 4.5
    }
}

/// A neutral surface at OKLCH `lightness` tinted `chroma` at `hue_degrees`,
/// with as much of the tint as keeps `text` readable on it. At no tint it
/// is the authored grey, which holds by the untinted floors.
pub(super) fn tinted_surface(lightness: f32, chroma: f32, hue_degrees: f32, text: &Text) -> Color {
    let mut chroma = chroma;
    loop {
        let surface = oklch(lightness, chroma, hue_degrees);
        if chroma <= 0.0 || text.reads_on(surface) {
            return surface;
        }
        chroma = (chroma - CHROMA_STEP).max(0.0);
    }
}

/// `background` (a mix the design gives as a share, such as the selected
/// row) as authored when `text` reads on it, else with its lightness moved
/// away from the text until it does; `dark` says which way that is.
pub(super) fn readable_background(background: Color, dark: bool, text: &Text) -> Color {
    let (lightness, chroma, hue_degrees) = lch(background);
    let step = if dark {
        -LIGHTNESS_STEP
    } else {
        LIGHTNESS_STEP
    };
    reach_floor(lightness, chroma, hue_degrees, step, |candidate| {
        text.reads_on(candidate)
    })
}

/// A link at `lightness`, `chroma` and `hue_degrees` as authored when it
/// keeps 4.5:1 on every one of `surfaces`, else with its lightness moved
/// away from them (lighter on dark themes) until it does.
pub(super) fn link(
    lightness: f32,
    chroma: f32,
    hue_degrees: f32,
    dark: bool,
    surfaces: &[Color],
) -> Color {
    let step = if dark {
        LIGHTNESS_STEP
    } else {
        -LIGHTNESS_STEP
    };
    reach_floor(lightness, chroma, hue_degrees, step, |candidate| {
        surfaces
            .iter()
            .all(|surface| contrast(candidate, *surface) >= 4.5)
    })
}

/// `colour`'s OKLCH lightness, chroma and hue in degrees.
fn lch(colour: Color) -> (f32, f32, f32) {
    let [lightness, a, b, _] = colour.to_oklab();
    (
        lightness,
        a.hypot(b),
        b.atan2(a).to_degrees().rem_euclid(360.0),
    )
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
        text.reads_on(background) && contrast(text.link, background) >= 4.5
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
    let (lightness, chroma, hue_degrees) = lch(role);
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
