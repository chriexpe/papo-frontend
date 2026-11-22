//! The colour tokens (`docs/design.md` §4): the four themes' neutral
//! lightnesses, the accent and feedback colours, and the mixes built from
//! them, authored in OKLCH and converted to Rune colours here, with the
//! WCAG contrast the floors are tested with.
//!
//! Not built yet: the user's background colour, strength and grain (every
//! neutral below is untinted) and the accent choice (the default blue only).

use rune::{Color, oklch};

use super::CodeInk;

/// The colour theme. It is a setting of its own, not the system's light or
/// dark preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
struct Lightness {
    surface_rail: f32,
    surface_sidebar: f32,
    surface_hover: f32,
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
pub(super) const ACCENT_HUE_DEGREES: f32 = 256.0;
/// Every accent's OKLCH chroma.
const ACCENT_CHROMA: f32 = 0.18;
/// The opacity of the accent behind selected text.
const TEXT_SELECTION_ALPHA: f32 = 0.3;
/// A link's OKLCH chroma; it takes the accent's hue.
const LINK_CHROMA: f32 = 0.15;
/// The mention red's OKLCH hue in degrees (`docs/design.md` §4).
const MENTION_HUE_DEGREES: f32 = 25.0;
/// The mention red's OKLCH chroma, as the mock draws it.
const MENTION_CHROMA: f32 = 0.20;
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
/// The mention-row marker's OKLCH hue in degrees: amber (`docs/design.md`
/// §4).
const MENTION_ROW_HUE_DEGREES: f32 = 82.0;
/// The mention-row marker's OKLCH chroma, as the design project draws it.
const MENTION_ROW_CHROMA: f32 = 0.15;
/// The opacity of the black that dims the app behind a popup.
const SCRIM_ALPHA: f32 = 0.5;
/// The opacity of a popup's black shadow.
const POPUP_SHADOW_ALPHA: f32 = 0.45;

/// A theme's colours, ready to draw with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// The server rail.
    pub(crate) surface_rail: Color,
    /// The channel list.
    pub(crate) surface_sidebar: Color,
    /// A row under the pointer.
    pub(crate) surface_hover: Color,
    /// The selected channel: the accent mixed into the sidebar.
    pub(crate) surface_selected: Color,
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
    /// Errors: a feedback line, an invalid field's outline, a button that
    /// destroys something.
    pub(crate) danger: Color,
    /// Text drawn on the danger colour: black or white, whichever contrasts
    /// more.
    pub(crate) danger_on: Color,
    /// Warnings: a feedback line that lets the person go on.
    pub(crate) warning: Color,
    /// Success: a met requirement.
    pub(crate) success: Color,
    /// Mentions of you: the badges' count pill.
    pub(crate) mention: Color,
    /// The count drawn on the mention colour: white, by Christian's choice
    /// (`docs/design.md` §3, Badges), held to 3:1.
    pub(crate) mention_on: Color,
    /// A mention's chip, behind its text in the link colour: the accent
    /// mixed into `surface.content` in OKLab, 20 % dark and 12 % light.
    pub(crate) accent_soft: Color,
    /// The bar on the left of a message that mentions you: amber.
    pub(crate) mention_bar: Color,
    /// The background of a message that mentions you: the bar's amber mixed
    /// into `surface.content` in OKLab, 4 % dark and 13 % light, the most
    /// every theme's text floors allow (Christian, 2026-11-21).
    pub(crate) mention_row: Color,
    /// Highlighted code's token colours.
    pub(crate) code: CodeInk,
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
        let danger = oklch(danger_lightness, DANGER_CHROMA, DANGER_HUE_DEGREES);
        let mention_lightness = if dark { 0.66 } else { 0.60 };
        let mention = oklch(mention_lightness, MENTION_CHROMA, MENTION_HUE_DEGREES);
        let success_lightness = if dark { 0.76 } else { 0.50 };
        let (warning_lightness, warning_hue) = if dark { (0.82, 80.0) } else { (0.50, 65.0) };
        let sidebar = neutral(lightness.surface_sidebar);
        let content = neutral(lightness.surface_content);
        let code = neutral(lightness.surface_code);
        let mention_bar_lightness = if dark { 0.78 } else { 0.70 };
        let mention_bar = oklch(
            mention_bar_lightness,
            MENTION_ROW_CHROMA,
            MENTION_ROW_HUE_DEGREES,
        );
        let selected_share = if dark { 0.10 } else { 0.09 };
        Palette {
            surface_rail: neutral(lightness.surface_rail),
            surface_sidebar: sidebar,
            surface_hover: neutral(lightness.surface_hover),
            surface_selected: mix_oklab(sidebar, accent, selected_share),
            surface_content: content,
            surface_pill: neutral(lightness.surface_pill),
            surface_code: code,
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
            danger,
            danger_on: text_on(danger),
            warning: oklch(warning_lightness, WARNING_CHROMA, warning_hue),
            success: oklch(success_lightness, SUCCESS_CHROMA, SUCCESS_HUE_DEGREES),
            mention,
            mention_on: Color::WHITE,
            accent_soft: mix_oklab(content, accent, if dark { 0.20 } else { 0.12 }),
            mention_bar,
            mention_row: mix_oklab(content, mention_bar, if dark { 0.04 } else { 0.13 }),
            code: CodeInk::new(dark, code),
            scrim: Color::BLACK.alpha(SCRIM_ALPHA),
            popup_shadow: Color::BLACK.alpha(POPUP_SHADOW_ALPHA),
        }
    }
}

/// `share` of the way from `base` to `tint`, in `OKLab`, so the mix keeps
/// close to the base's perceived lightness and the text on it stays
/// readable.
fn mix_oklab(base: Color, tint: Color, share: f32) -> Color {
    let (from, to) = (base.to_oklab(), tint.to_oklab());
    let mut mixed = [0.0; 4];
    for (channel, (a, b)) in mixed.iter_mut().zip(from.iter().zip(to.iter())) {
        *channel = a + (b - a) * share;
    }
    Color::from_oklab(mixed)
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
pub(super) fn contrast(a: Color, b: Color) -> f32 {
    let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}
