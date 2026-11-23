//! The colour tokens (`docs/design.md` §4): the four themes' neutral
//! lightnesses, the accent, feedback, presence and search colours, and the
//! mixes built from them, authored in OKLCH and converted to Rune colours
//! here, with the WCAG contrast the floors are tested with. Highlights are
//! fitted to their floors by `floors`.
//!
//! The neutrals take the user's background colour (`background`); grain is
//! drawn over the surfaces, not mixed into them.

use rune::{Color, oklch};

use super::CodeInk;
use super::background::{Accent, Background, BackgroundColour, ChromeFill};
use super::floors::{self, Text, highlight, in_gamut};

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
/// Available's OKLCH hue, chroma, and lightness on dark and light themes
/// (§4, Other colours added for D5).
pub(super) const PRESENCE_ONLINE: Oklch = Oklch {
    hue_degrees: 150.0,
    chroma: 0.17,
    dark: 0.72,
    light: 0.57,
};
/// Away's OKLCH hue, chroma and lightnesses; a search match's underline
/// takes the same.
pub(super) const PRESENCE_AWAY: Oklch = Oklch {
    hue_degrees: 82.0,
    chroma: 0.15,
    dark: 0.78,
    light: 0.59,
};
/// The opacity of the black that dims the app behind a popup.
const SCRIM_ALPHA: f32 = 0.5;
/// The opacity of a popup's black shadow.
const POPUP_SHADOW_ALPHA: f32 = 0.45;
/// How far a light theme's settings tile moves from the pill towards
/// `text.primary`, in linear light as the mock does (§8, Open). On dark
/// themes nothing lighter than the pill keeps 13:1, so the tile is the pill
/// and the panel behind it the sidebar (Christian, 2026-11-23).
const LIGHT_TILE_SHARE: f32 = 0.035;
/// `accent.soft`'s share of the accent, mixed into `surface.content` in
/// OKLab: 20 % on dark themes, 12 % on light ones (§4). The link is fitted
/// to it, not the other way round.
const ACCENT_SOFT_SHARE: (f32, f32) = (0.20, 0.12);
/// The accent's share of `surface.selected`, mixed in OKLab: 10 % on dark
/// themes, 9 % on light ones (§4), a target `floors` gives way from.
const SELECTED_SHARE: (f32, f32) = (0.10, 0.09);
/// A link's target OKLCH lightness on dark and light themes; `floors`
/// moves it where a surface it sits on needs more (Teal and Green, which
/// read brighter, come out darker on light themes).
const LINK_LIGHTNESS: (f32, f32) = (0.76, 0.50);
/// OLED's sidebar is black, where the mix shows nothing (1.01:1), so its
/// selected row mixes into this lightness instead: the result sits at
/// `surface.hover`'s lightness with the accent's tint, as Graphite's does
/// (Christian, 2026-11-23: 1.15:1 against the pane).
const OLED_SELECTED_BASE_LIGHTNESS: f32 = 0.139;

/// A colour authored as an OKLCH hue and chroma, at one lightness on dark
/// themes and another on light ones.
pub(super) struct Oklch {
    pub(super) hue_degrees: f32,
    pub(super) chroma: f32,
    pub(super) dark: f32,
    pub(super) light: f32,
}

impl Oklch {
    /// The colour at the theme's lightness. Where sRGB cannot hold the
    /// chroma there (Available's 0.17 at L 0.57 holds 0.158), only the
    /// chroma is lowered, as §4's highlights do, so the lightness the
    /// contrast rests on stays as authored.
    fn on(&self, dark: bool) -> Color {
        let lightness = if dark { self.dark } else { self.light };
        let chroma = in_gamut(lightness, self.chroma, self.hue_degrees);
        oklch(lightness, chroma, self.hue_degrees)
    }
}

/// A theme's colours, ready to draw with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    /// The server rail: `rail_fill`'s first colour, the one contrast is
    /// measured on.
    pub(crate) surface_rail: Color,
    /// The channel list: `sidebar_fill`'s first colour.
    pub(crate) surface_sidebar: Color,
    /// What the rail is filled with: a gradient with two or three
    /// background colours.
    pub(crate) rail_fill: ChromeFill,
    /// What the channel and DM lists are filled with.
    pub(crate) sidebar_fill: ChromeFill,
    /// A settings row's tile (§8): the pill, on light themes a step
    /// towards the text.
    pub(crate) surface_tile: Color,
    /// The settings panel behind the tiles: the sidebar on dark themes,
    /// the pill on light ones.
    pub(crate) surface_panel: Color,
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
    /// mixed into `surface.content` in OKLab, 20 % dark and 10 % light.
    pub(crate) accent_soft: Color,
    /// `highlight.mention`: the background of a message that mentions you,
    /// with no bar beside it (§4, Highlights: the rule at the mention hue).
    pub(crate) highlight_mention: Color,
    /// Keyboard focus: a 2 pt solid ring in the accent.
    pub(crate) focus_ring: Color,
    /// Presence marks (§10, Presence): Available, Away, Busy (the badge
    /// red) and Offline (`text.muted`). Drawn from D5b (members pane,
    /// profile card); until then only the contrast tests read them.
    pub(crate) presence_online: Color,
    pub(crate) presence_away: Color,
    pub(crate) presence_busy: Color,
    pub(crate) presence_offline: Color,
    /// The 2 pt underline under a matched word in search results. Drawn
    /// from D5c (search); until then only the contrast tests read it.
    pub(crate) search_match: Color,
    /// Highlighted code's token colours.
    pub(crate) code: CodeInk,
    /// What dims the app behind a popup.
    pub(crate) scrim: Color,
    /// A popup's shadow.
    pub(crate) popup_shadow: Color,
}

impl Palette {
    /// `theme` with the default accent and background colour.
    #[cfg(test)]
    pub(crate) fn new(theme: Theme) -> Palette {
        Palette::with(theme, Accent::default(), &Background::default())
    }

    /// `theme` with the user's `accent` and `background`: the design's
    /// values as targets, each fitted to its floors (`floors`).
    pub(crate) fn with(theme: Theme, accent: Accent, background: &Background) -> Palette {
        let lightness = theme.lightness();
        let dark = theme.is_dark();
        let pick = |pair: (f32, f32)| if dark { pair.0 } else { pair.1 };
        let accent_choice = accent;
        let accent_hue = accent.hue_degrees();
        let accent = oklch(pick((0.70, 0.555)), ACCENT_CHROMA, accent_hue);
        let danger = oklch(pick((0.74, 0.52)), DANGER_CHROMA, DANGER_HUE_DEGREES);
        let mention = oklch(pick((0.66, 0.60)), MENTION_CHROMA, MENTION_HUE_DEGREES);
        let (warning_lightness, warning_hue) = if dark { (0.82, 80.0) } else { (0.50, 65.0) };
        let textual = |l: f32| background.neutral(l, true);
        let mut text = Text {
            primary: textual(lightness.text_primary),
            secondary: textual(lightness.text_secondary),
            muted: textual(lightness.text_muted),
            // Fitted below, once the surfaces it sits on are known.
            link: oklch(pick(LINK_LIGHTNESS), LINK_CHROMA, accent_hue),
        };
        // A surface text sits on: the tint, as much of it as the text allows.
        let surface_with = |colour: Option<BackgroundColour>, l: f32| {
            let (chroma, hue) = background.tint(colour, l, false);
            floors::tinted_surface(l, chroma, hue, &text)
        };
        let surface = |l: f32| surface_with(background.first(), l);
        let neutral = |l: f32| background.neutral(l, false);
        let rail_fill = ChromeFill::new(background, |colour| {
            surface_with(colour, lightness.surface_rail)
        });
        let sidebar_fill = ChromeFill::new(background, |colour| {
            surface_with(colour, lightness.surface_sidebar)
        });
        let sidebar = sidebar_fill.first();
        let content = surface(lightness.surface_content);
        let code = surface(lightness.surface_code);
        let pill = surface(lightness.surface_pill);
        let hover = surface(lightness.surface_hover);
        let selected_base = match theme {
            Theme::Oled => neutral(OLED_SELECTED_BASE_LIGHTNESS),
            _ => sidebar,
        };
        let selected = floors::readable_background(
            mix_oklab(selected_base, accent, pick(SELECTED_SHARE)),
            dark,
            &text,
        );
        let (surface_tile, surface_panel) = if dark {
            (pill, sidebar)
        } else {
            let tile = mix_linear(pill, text.primary, LIGHT_TILE_SHARE);
            (floors::readable_background(tile, dark, &text), pill)
        };
        let accent_soft = mix_oklab(content, accent, pick(ACCENT_SOFT_SHARE));
        text.link = floors::link(
            pick(LINK_LIGHTNESS),
            LINK_CHROMA,
            accent_hue,
            dark,
            &[content, pill, hover, sidebar, code, selected, accent_soft],
        );
        Palette {
            surface_rail: rail_fill.first(),
            surface_sidebar: sidebar,
            rail_fill,
            sidebar_fill,
            surface_tile,
            surface_panel,
            surface_hover: hover,
            surface_selected: selected,
            surface_content: content,
            surface_pill: pill,
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
            success: oklch(pick((0.76, 0.50)), SUCCESS_CHROMA, SUCCESS_HUE_DEGREES),
            mention,
            mention_on: Color::WHITE,
            accent_soft,
            highlight_mention: highlight(theme, MENTION_HIGHLIGHT_HUE_DEGREES, &text),
            focus_ring: accent,
            presence_online: PRESENCE_ONLINE.on(dark),
            presence_away: PRESENCE_AWAY.on(dark),
            presence_busy: mention,
            presence_offline: text.muted,
            search_match: PRESENCE_AWAY.on(dark),
            code: CodeInk::new(dark, code, accent_choice),
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

/// `share` of the way from `base` to `tint` in linear light.
fn mix_linear(base: Color, tint: Color, share: f32) -> Color {
    Color {
        r: base.r + (tint.r - base.r) * share,
        g: base.g + (tint.g - base.g) * share,
        b: base.b + (tint.b - base.b) * share,
        a: base.a,
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
pub(super) fn contrast(a: Color, b: Color) -> f32 {
    let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}
