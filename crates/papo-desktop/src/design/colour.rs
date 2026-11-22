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

    /// Where a highlight starts (`docs/design.md` §4, Highlights): the
    /// theme's background lightness and its fixed chroma.
    fn highlight_start(self) -> (f32, f32) {
        match self {
            Theme::Oled => (0.170, 0.040),
            Theme::Graphite => (0.262, 0.030),
            Theme::Light => (0.962, 0.035),
            Theme::SoftLight => (0.942, 0.040),
        }
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
/// The mention highlight's default OKLCH hue in degrees: amber
/// (`docs/design.md` §4; the user's to change once Settings exist).
const MENTION_HIGHLIGHT_HUE_DEGREES: f32 = 75.0;
/// How far a highlight's lightness moves per step while it looks for the
/// text floors.
const HIGHLIGHT_STEP: f32 = 0.002;
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
    /// `highlight.mention`: the background of a message that mentions you,
    /// with no bar beside it (§4, Highlights: the rule at the mention hue).
    pub(crate) highlight_mention: Color,
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
        let text = Text {
            primary: neutral(lightness.text_primary),
            secondary: neutral(lightness.text_secondary),
            muted: neutral(lightness.text_muted),
            link: oklch(link_lightness, LINK_CHROMA, ACCENT_HUE_DEGREES),
        };
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
            text_primary: text.primary,
            text_secondary: text.secondary,
            text_muted: text.muted,
            accent,
            accent_on: text_on(accent),
            text_selection: accent.alpha(TEXT_SELECTION_ALPHA),
            link: text.link,
            danger,
            danger_on: text_on(danger),
            warning: oklch(warning_lightness, WARNING_CHROMA, warning_hue),
            success: oklch(success_lightness, SUCCESS_CHROMA, SUCCESS_HUE_DEGREES),
            mention,
            mention_on: Color::WHITE,
            accent_soft: mix_oklab(content, accent, if dark { 0.20 } else { 0.12 }),
            highlight_mention: highlight(theme, MENTION_HIGHLIGHT_HUE_DEGREES, &text),
            code: CodeInk::new(dark, code),
            scrim: Color::BLACK.alpha(SCRIM_ALPHA),
            popup_shadow: Color::BLACK.alpha(POPUP_SHADOW_ALPHA),
        }
    }
}

/// The text a highlight must keep readable.
struct Text {
    primary: Color,
    secondary: Color,
    muted: Color,
    link: Color,
}

/// A highlight at `hue_degrees` (§4, Highlights): from the theme's
/// background lightness and fixed chroma (lowered only where sRGB cannot
/// hold it), the lightness moves away from the text until `text.primary`
/// keeps 13:1, `text.secondary` 7:1, and `text.muted` and links 4.5:1.
fn highlight(theme: Theme, hue_degrees: f32, text: &Text) -> Color {
    let (start, chroma) = theme.highlight_start();
    let step = if theme.is_dark() {
        -HIGHLIGHT_STEP
    } else {
        HIGHLIGHT_STEP
    };
    let readable = |background: Color| {
        contrast(text.primary, background) >= 13.0
            && contrast(text.secondary, background) >= 7.0
            && contrast(text.muted, background) >= 4.5
            && contrast(text.link, background) >= 4.5
    };
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
fn in_gamut(lightness: f32, chroma: f32, hue_degrees: f32) -> f32 {
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
