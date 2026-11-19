//! Papo's design tokens: colours, type, radii and spacing.
//!
//! Every value is Papo's own design decision; nothing here comes from Rune's
//! default theme. Colours are authored as OKLCH lightness per theme and
//! converted to Rune colours here, so no view writes a colour by hand. Views
//! own layout; this module owns only the values they lay out with.
//!
//! Not built yet: the user's background colour, strength and grain (every
//! neutral below is untinted), the accent choice (the default blue only), and
//! the text-size, roundness and spacing multipliers (all at ×1).

use rune::{Color, TextStyle, oklch};

/// The colour theme. It is a setting of its own, not the system's light or
/// dark preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Theme {
    /// True black on every large surface.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Appearance settings that choose it are not built yet"
        )
    )]
    Oled,
    Graphite,
    Light,
    /// Every surface a step below white, so white pills stand out.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Appearance settings that choose it are not built yet"
        )
    )]
    SoftLight,
}

impl Theme {
    /// The theme used until the user picks one: Graphite or Light, following
    /// the system's preference.
    pub(crate) fn following_system(system_dark: bool) -> Theme {
        if system_dark {
            Theme::Graphite
        } else {
            Theme::Light
        }
    }

    fn is_dark(self) -> bool {
        matches!(self, Theme::Oled | Theme::Graphite)
    }

    /// The OKLCH lightness (0–1) of each neutral token in this theme.
    fn lightness(self) -> Lightness {
        match self {
            Theme::Oled => Lightness {
                surface_content: 0.000,
                surface_pill: 0.175,
                line_pill: 0.255,
                text_primary: 0.965,
                text_muted: 0.645,
            },
            Theme::Graphite => Lightness {
                surface_content: 0.252,
                surface_pill: 0.270,
                line_pill: 0.360,
                text_primary: 0.955,
                text_muted: 0.665,
            },
            Theme::Light => Lightness {
                surface_content: 1.000,
                surface_pill: 1.000,
                line_pill: 0.905,
                text_primary: 0.215,
                text_muted: 0.520,
            },
            Theme::SoftLight => Lightness {
                surface_content: 0.975,
                surface_pill: 1.000,
                line_pill: 0.890,
                text_primary: 0.215,
                text_muted: 0.505,
            },
        }
    }
}

/// OKLCH lightness of the neutral tokens a theme defines.
struct Lightness {
    surface_content: f32,
    surface_pill: f32,
    line_pill: f32,
    text_primary: f32,
    text_muted: f32,
}

/// The default accent, Blue, as an OKLCH hue in degrees.
const ACCENT_HUE_DEGREES: f32 = 256.0;
/// Every accent's OKLCH chroma.
const ACCENT_CHROMA: f32 = 0.18;
/// The opacity of the accent behind selected text.
const TEXT_SELECTION_ALPHA: f32 = 0.3;

/// A theme's colours, ready to draw with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// The message list.
    pub(crate) surface_content: Color,
    /// Floating pills: the header pills and the composer.
    pub(crate) surface_pill: Color,
    /// A pill's outline.
    pub(crate) line_pill: Color,
    /// Message text, names, unread channels.
    pub(crate) text_primary: Color,
    /// Timestamps, labels, placeholders.
    pub(crate) text_muted: Color,
    /// The accent: the caret.
    pub(crate) accent: Color,
    /// Selected text's highlight: the accent, translucent.
    pub(crate) text_selection: Color,
}

impl Palette {
    pub(crate) fn new(theme: Theme) -> Palette {
        let lightness = theme.lightness();
        let neutral = |l: f32| oklch(l, 0.0, 0.0);
        let accent_lightness = if theme.is_dark() { 0.70 } else { 0.555 };
        let accent = oklch(accent_lightness, ACCENT_CHROMA, ACCENT_HUE_DEGREES);
        Palette {
            surface_content: neutral(lightness.surface_content),
            surface_pill: neutral(lightness.surface_pill),
            line_pill: neutral(lightness.line_pill),
            text_primary: neutral(lightness.text_primary),
            text_muted: neutral(lightness.text_muted),
            accent,
            text_selection: accent.alpha(TEXT_SELECTION_ALPHA),
        }
    }
}

/// `type.body` in points: message text and channel names. Every other text
/// size is a ratio of it.
const TYPE_BODY_PT: f32 = 14.0;
/// Body text's line height, as a multiple of its size.
const BODY_LINE_HEIGHT: f32 = 1.45;

/// Message text and channel names, at weight 400.
pub(crate) const TYPE_BODY: TextStyle =
    TextStyle::new(TYPE_BODY_PT, 400.0).lh(TYPE_BODY_PT * BODY_LINE_HEIGHT);

/// `radius.composer`, the message field's corner radius in points.
pub(crate) const RADIUS_COMPOSER_PT: f32 = 16.0;
/// `space.lg` in points.
pub(crate) const SPACE_LG_PT: f32 = 12.0;
/// The composer pill's height in points.
pub(crate) const COMPOSER_HEIGHT_PT: f32 = 44.0;
/// The width of a pill's outline in points.
pub(crate) const PILL_OUTLINE_PT: f32 = 1.0;

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: [Theme; 4] = [Theme::Oled, Theme::Graphite, Theme::Light, Theme::SoftLight];

    /// WCAG 2 contrast ratio. Rune colours are linear sRGB, so the relative
    /// luminance is a weighted sum of the channels as they are.
    fn contrast(a: Color, b: Color) -> f32 {
        let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        let (light, dark) = {
            let (la, lb) = (luminance(a), luminance(b));
            if la > lb { (la, lb) } else { (lb, la) }
        };
        (light + 0.05) / (dark + 0.05)
    }

    #[test]
    fn text_meets_its_contrast_floor_on_every_surface_it_sits_on() {
        for theme in THEMES {
            let palette = Palette::new(theme);
            for (surface, name) in [
                (palette.surface_content, "content"),
                (palette.surface_pill, "pill"),
            ] {
                let primary = contrast(palette.text_primary, surface);
                let muted = contrast(palette.text_muted, surface);
                assert!(
                    primary >= 13.0,
                    "{theme:?} text.primary on {name}: {primary:.2}:1"
                );
                assert!(muted >= 4.5, "{theme:?} text.muted on {name}: {muted:.2}:1");
            }
        }
    }

    #[test]
    fn neutral_surfaces_are_grey() {
        for theme in THEMES {
            let pill = Palette::new(theme).surface_pill;
            assert!(
                (pill.r - pill.g).abs() < 1e-4 && (pill.g - pill.b).abs() < 1e-4,
                "{theme:?}: {pill:?}"
            );
        }
    }

    #[test]
    fn oled_content_is_true_black() {
        let content = Palette::new(Theme::Oled).surface_content;
        assert_eq!((content.r, content.g, content.b), (0.0, 0.0, 0.0));
    }
}
