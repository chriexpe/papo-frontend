//! The four themes (`docs/design.md` §4, Themes): which a user picks and
//! the OKLCH lightness each gives the neutral tokens, before any background
//! colour tints them.
//!
//! Owns the themes and their lightness tables; the tokens built from them
//! are `colour`'s (`Palette`).

/// The colour theme. It is a setting of its own, not the system's light or
/// dark preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Theme {
    /// True black on every large surface.
    Oled,
    Graphite,
    Light,
    /// Every surface a step below white, so white pills stand out.
    SoftLight,
}

impl Theme {
    /// Every theme, in the settings' order.
    pub(crate) const ALL: [Theme; 4] =
        [Theme::Oled, Theme::Graphite, Theme::Light, Theme::SoftLight];

    /// The theme used until the user picks one: Graphite or Light, following
    /// the system's preference.
    pub(crate) fn following_system(system_dark: bool) -> Theme {
        if system_dark {
            Theme::Graphite
        } else {
            Theme::Light
        }
    }

    pub(crate) fn is_dark(self) -> bool {
        matches!(self, Theme::Oled | Theme::Graphite)
    }

    /// Where a highlight starts (`docs/design.md` §4, Highlights): the
    /// theme's background lightness and its fixed chroma.
    pub(super) fn highlight_start(self) -> (f32, f32) {
        match self {
            Theme::Oled => (0.170, 0.040),
            Theme::Graphite => (0.262, 0.030),
            Theme::Light => (0.962, 0.035),
            Theme::SoftLight => (0.942, 0.040),
        }
    }

    /// The OKLCH lightness (0–1) of each neutral token in this theme.
    pub(super) fn lightness(self) -> Lightness {
        match self {
            Theme::Oled => Lightness {
                surface_rail: 0.000,
                surface_sidebar: 0.000,
                surface_hover: 0.195,
                surface_content: 0.000,
                surface_pill: 0.175,
                surface_code: 0.135,
                line_subtle: 0.225,
                line_strong: 0.320,
                line_pill: 0.255,
                text_primary: 0.965,
                text_secondary: 0.800,
                text_muted: 0.645,
            },
            Theme::Graphite => Lightness {
                surface_rail: 0.205,
                surface_sidebar: 0.228,
                surface_hover: 0.275,
                surface_content: 0.252,
                surface_pill: 0.270,
                surface_code: 0.222,
                line_subtle: 0.310,
                line_strong: 0.390,
                line_pill: 0.360,
                text_primary: 0.955,
                text_secondary: 0.800,
                text_muted: 0.665,
            },
            Theme::Light => Lightness {
                surface_rail: 0.952,
                surface_sidebar: 0.972,
                surface_hover: 0.940,
                surface_content: 1.000,
                surface_pill: 1.000,
                surface_code: 0.968,
                line_subtle: 0.918,
                line_strong: 0.840,
                line_pill: 0.905,
                text_primary: 0.215,
                text_secondary: 0.400,
                text_muted: 0.520,
            },
            Theme::SoftLight => Lightness {
                surface_rail: 0.925,
                surface_sidebar: 0.950,
                surface_hover: 0.918,
                surface_content: 0.975,
                surface_pill: 1.000,
                surface_code: 0.945,
                line_subtle: 0.900,
                line_strong: 0.820,
                line_pill: 0.890,
                text_primary: 0.215,
                text_secondary: 0.395,
                text_muted: 0.505,
            },
        }
    }
}

/// OKLCH lightness of the neutral tokens a theme defines.
pub(super) struct Lightness {
    pub(super) surface_rail: f32,
    pub(super) surface_sidebar: f32,
    pub(super) surface_hover: f32,
    pub(super) surface_content: f32,
    pub(super) surface_pill: f32,
    pub(super) surface_code: f32,
    pub(super) line_subtle: f32,
    pub(super) line_strong: f32,
    pub(super) line_pill: f32,
    pub(super) text_primary: f32,
    pub(super) text_secondary: f32,
    pub(super) text_muted: f32,
}
