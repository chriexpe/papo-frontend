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
//!
//! The feedback colours (danger, warning, success) and the sizes of the server
//! and account screens come from the design project's "Onboarding and Sign In"
//! page, which `docs/design.md` §7 was settled from.

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
                surface_code: 0.135,
                line_subtle: 0.225,
                line_strong: 0.320,
                line_pill: 0.255,
                text_primary: 0.965,
                text_secondary: 0.800,
                text_muted: 0.645,
            },
            Theme::Graphite => Lightness {
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
struct Lightness {
    surface_content: f32,
    surface_pill: f32,
    surface_code: f32,
    line_subtle: f32,
    line_strong: f32,
    line_pill: f32,
    text_primary: f32,
    text_secondary: f32,
    text_muted: f32,
}

/// The default accent, Blue, as an OKLCH hue in degrees.
const ACCENT_HUE_DEGREES: f32 = 256.0;
/// Every accent's OKLCH chroma.
const ACCENT_CHROMA: f32 = 0.18;
/// The opacity of the accent behind selected text.
const TEXT_SELECTION_ALPHA: f32 = 0.3;
/// A link's OKLCH chroma; it takes the accent's hue.
const LINK_CHROMA: f32 = 0.15;
/// The danger colour's OKLCH hue in degrees: the mention red's.
const DANGER_HUE_DEGREES: f32 = 25.0;
/// The success colour's OKLCH hue in degrees: the online green's.
const SUCCESS_HUE_DEGREES: f32 = 150.0;
/// The danger colour's OKLCH chroma: washed out, not vivid.
const DANGER_CHROMA: f32 = 0.11;
/// The success colour's OKLCH chroma.
const SUCCESS_CHROMA: f32 = 0.09;
/// The warning colour's OKLCH chroma; its hue moves with the theme.
const WARNING_CHROMA: f32 = 0.09;
/// The opacity of the black that dims the app behind a popup.
const SCRIM_ALPHA: f32 = 0.5;
/// The opacity of a popup's black shadow.
const POPUP_SHADOW_ALPHA: f32 = 0.45;

/// A theme's colours, ready to draw with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// The message list.
    pub(crate) surface_content: Color,
    /// Floating pills: the header pills and the composer.
    pub(crate) surface_pill: Color,
    /// Code blocks, and the inside of a form field.
    pub(crate) surface_code: Color,
    /// Pane edges and dividers; a popup's outline.
    pub(crate) line_subtle: Color,
    /// Emphasised edges: a form field's outline.
    pub(crate) line_strong: Color,
    /// A pill's outline.
    pub(crate) line_pill: Color,
    /// Message text, names, unread channels.
    pub(crate) text_primary: Color,
    /// Read channels, reactions, field labels, secondary links.
    pub(crate) text_secondary: Color,
    /// Timestamps, labels, placeholders.
    pub(crate) text_muted: Color,
    /// The accent: the caret, the primary button, a focused field's outline.
    pub(crate) accent: Color,
    /// Text drawn on the accent: black or white, whichever contrasts more.
    pub(crate) accent_on: Color,
    /// Selected text's highlight: the accent, translucent.
    pub(crate) text_selection: Color,
    /// Links: the accent's hue, lighter on dark themes.
    pub(crate) link: Color,
    /// Errors: a feedback line, an invalid field's outline.
    pub(crate) danger: Color,
    /// Warnings: a feedback line that lets the person go on.
    pub(crate) warning: Color,
    /// Success: a met requirement.
    pub(crate) success: Color,
    /// What dims the app behind a popup.
    pub(crate) scrim: Color,
    /// A popup's shadow.
    pub(crate) popup_shadow: Color,
}

impl Palette {
    pub(crate) fn new(theme: Theme) -> Palette {
        let lightness = theme.lightness();
        let neutral = |l: f32| oklch(l, 0.0, 0.0);
        let dark = theme.is_dark();
        let accent_lightness = if dark { 0.70 } else { 0.555 };
        let accent = oklch(accent_lightness, ACCENT_CHROMA, ACCENT_HUE_DEGREES);
        let link_lightness = if dark { 0.76 } else { 0.50 };
        let danger_lightness = if dark { 0.74 } else { 0.52 };
        let success_lightness = if dark { 0.76 } else { 0.50 };
        let (warning_lightness, warning_hue) = if dark { (0.82, 80.0) } else { (0.50, 65.0) };
        Palette {
            surface_content: neutral(lightness.surface_content),
            surface_pill: neutral(lightness.surface_pill),
            surface_code: neutral(lightness.surface_code),
            line_subtle: neutral(lightness.line_subtle),
            line_strong: neutral(lightness.line_strong),
            line_pill: neutral(lightness.line_pill),
            text_primary: neutral(lightness.text_primary),
            text_secondary: neutral(lightness.text_secondary),
            text_muted: neutral(lightness.text_muted),
            accent,
            accent_on: text_on(accent),
            text_selection: accent.alpha(TEXT_SELECTION_ALPHA),
            link: oklch(link_lightness, LINK_CHROMA, ACCENT_HUE_DEGREES),
            danger: oklch(danger_lightness, DANGER_CHROMA, DANGER_HUE_DEGREES),
            warning: oklch(warning_lightness, WARNING_CHROMA, warning_hue),
            success: oklch(success_lightness, SUCCESS_CHROMA, SUCCESS_HUE_DEGREES),
            scrim: Color::BLACK.alpha(SCRIM_ALPHA),
            popup_shadow: Color::BLACK.alpha(POPUP_SHADOW_ALPHA),
        }
    }
}

/// Black or white, whichever contrasts more with `background`.
fn text_on(background: Color) -> Color {
    let (black, white) = (Color::BLACK, Color::WHITE);
    if contrast(black, background) >= contrast(white, background) {
        black
    } else {
        white
    }
}

/// WCAG 2 contrast ratio. Rune colours are linear sRGB, so the relative
/// luminance is a weighted sum of the channels as they are.
fn contrast(a: Color, b: Color) -> f32 {
    let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// `type.body` in points: message text and channel names. Every other text
/// size is a ratio of it.
const TYPE_BODY_PT: f32 = 14.0;
/// Text's line height, as a multiple of its size.
const BODY_LINE_HEIGHT: f32 = 1.45;

/// A text style `ratio` times `type.body`, at `weight`.
const fn type_scaled(ratio: f32, weight: f32) -> TextStyle {
    let size = TYPE_BODY_PT * ratio;
    TextStyle::new(size, weight).lh(size * BODY_LINE_HEIGHT)
}

/// Weight 400: running text.
const WEIGHT_REGULAR: f32 = 400.0;
/// Weight 500: selected and secondary emphasis.
const WEIGHT_MEDIUM: f32 = 500.0;
/// Weight 600: names, titles, unread channels.
const WEIGHT_SEMIBOLD: f32 = 600.0;

/// Message text and channel names, at weight 400.
pub(crate) const TYPE_BODY: TextStyle = type_scaled(1.0, WEIGHT_REGULAR);
/// `type.body` at weight 600: a primary button's label.
pub(crate) const TYPE_BODY_STRONG: TextStyle = type_scaled(1.0, WEIGHT_SEMIBOLD);
/// `type.small` at weight 500: field labels, links under a form.
pub(crate) const TYPE_SMALL_MEDIUM: TextStyle = type_scaled(12.0 / 13.0, WEIGHT_MEDIUM);
/// `type.small` at weight 400: a feedback line.
pub(crate) const TYPE_SMALL: TextStyle = type_scaled(12.0 / 13.0, WEIGHT_REGULAR);
/// `type.caption`: sub-lines, a server's address, a field's hint.
pub(crate) const TYPE_CAPTION: TextStyle = type_scaled(11.0 / 13.0, WEIGHT_REGULAR);
/// `type.display` at weight 600: a screen's title, a large server initial.
pub(crate) const TYPE_DISPLAY: TextStyle = type_scaled(18.0 / 13.0, WEIGHT_SEMIBOLD);

/// `radius.composer`, the message field's corner radius in points.
pub(crate) const RADIUS_COMPOSER_PT: f32 = 16.0;
/// `space.sm` in points.
pub(crate) const SPACE_SM_PT: f32 = 4.0;
/// `space.md` in points.
pub(crate) const SPACE_MD_PT: f32 = 8.0;
/// `space.lg` in points.
pub(crate) const SPACE_LG_PT: f32 = 12.0;
/// `space.xl` in points.
pub(crate) const SPACE_XL_PT: f32 = 16.0;
/// `space.xxl` in points.
pub(crate) const SPACE_XXL_PT: f32 = 20.0;
/// The composer pill's height in points.
pub(crate) const COMPOSER_HEIGHT_PT: f32 = 44.0;
/// The width of a pill's outline in points.
pub(crate) const PILL_OUTLINE_PT: f32 = 1.0;
/// `radius.control` in points: small controls inside a field.
pub(crate) const RADIUS_CONTROL_PT: f32 = 8.0;
/// `radius.pill`, a header pill's corner radius in points.
pub(crate) const RADIUS_PILL_PT: f32 = 14.0;
/// The header pills' height in points.
pub(crate) const HEADER_PILL_HEIGHT_PT: f32 = 36.0;

/// The server and account screens' column width, in points, in both hosts.
pub(crate) const FORM_WIDTH_PT: f32 = 340.0;
/// A form field's and a primary button's height, in points.
pub(crate) const FIELD_HEIGHT_PT: f32 = 40.0;
/// A form field's and a primary button's corner radius, in points.
pub(crate) const RADIUS_FIELD_PT: f32 = 10.0;
/// The height of a button inside a field (Show, Hide), in points.
pub(crate) const FIELD_BUTTON_HEIGHT_PT: f32 = 28.0;
/// The width of a field's outline, in points.
pub(crate) const FIELD_OUTLINE_PT: f32 = 1.0;
/// The opacity of a field that cannot take input while a request runs.
pub(crate) const FIELD_BUSY_OPACITY: f32 = 0.6;
/// The opacity of a button that cannot be pressed yet.
pub(crate) const BUTTON_DISABLED_OPACITY: f32 = 0.45;
/// The server's icon above the account steps, in points square.
pub(crate) const SERVER_ICON_PT: f32 = 56.0;
/// The server icon's corner radius, in points.
pub(crate) const RADIUS_SERVER_ICON_PT: f32 = 16.0;
/// A busy button's spinner, in points square.
pub(crate) const SPINNER_PT: f32 = 14.0;
/// The time a busy button's spinner takes to turn once, in seconds.
pub(crate) const SPINNER_PERIOD_S: f64 = 0.8;
/// The popup that hosts the server and account screens in the app, in points.
pub(crate) const POPUP_SIZE_PT: (f32, f32) = (460.0, 580.0);
/// The popup's padding, in points.
pub(crate) const POPUP_PADDING_PT: f32 = 32.0;
/// The popup's corner radius, in points.
pub(crate) const RADIUS_POPUP_PT: f32 = 16.0;
/// The popup's close button, in points square.
pub(crate) const POPUP_CLOSE_PT: f32 = 32.0;
/// The popup's close button's inset from the popup's corner, in points.
pub(crate) const POPUP_CLOSE_INSET_PT: f32 = 10.0;
/// The popup's shadow: its drop and its blur, in points.
pub(crate) const POPUP_SHADOW_PT: (f32, f32) = (24.0, 70.0);
/// A close cross inside its button, in points square.
pub(crate) const CROSS_PT: f32 = 16.0;
/// The close cross in a pill, in points square.
pub(crate) const PILL_CROSS_PT: f32 = 14.0;
/// A keyboard focus ring: its width and its gap from the control, in points.
pub(crate) const FOCUS_RING_PT: (f32, f32) = (2.0, 2.0);

#[cfg(test)]
mod tests;
