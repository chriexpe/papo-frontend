//! Papo's design tokens: colours (`colour`), type, radii and spacing.
//!
//! Every value is Papo's own design decision; nothing here comes from Rune's
//! default theme, and no view writes a colour or size by hand. Views own
//! layout; this module owns only the values they lay out with.
//!
//! Sizes are authored at the defaults (text size 14, spacing and roundness
//! ×1) and typed by what scales them: `Font` with the Text size setting,
//! `Space` with Spacing (gaps, paddings and pill heights), `Radius` with
//! Roundness, `TextMultiple` with the text size. A plain `_PT` number does
//! not scale: pane widths, icons, outlines and fixed row heights, so long
//! lists keep fixed rows (`docs/design.md` §5).
//!
//! The feedback colours (danger, warning, success) and the sizes of the server
//! and account screens come from the design project's "Onboarding and Sign In"
//! page, which `docs/design.md` §7 was settled from.

use rune::TextStyle;

/// `type.body` in points at the default text size: message text and
/// channel names. Every other text size is a ratio of it, and the Text size
/// setting replaces it (`Look::font`).
pub(crate) const TYPE_BODY_PT: f32 = 14.0;
/// Text's line height, as a multiple of its size.
const BODY_LINE_HEIGHT: f32 = 1.45;

/// A distance in points at spacing ×1: a gap, a padding, a pill's height.
/// `Look::space` gives it at the Spacing setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Space(pub(crate) f32);

/// A corner radius in points at roundness ×1. `Look::radius` gives it at
/// the Roundness setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Radius(pub(crate) f32);

/// A text style at the default text size. `Look::font` gives it at the
/// Text size setting, its line height scaled with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Font(pub(crate) TextStyle);

impl Font {
    /// The same style in the mono family.
    pub(crate) const fn mono(self) -> Font {
        Font(self.0.mono())
    }
}

/// A length that is a multiple of the text size (a list's marker column,
/// the Extra compact gutter). `Look::text_multiple` gives it in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextMultiple(pub(crate) f32);

/// A text style `ratio` times `type.body`, at `weight`.
const fn type_scaled(ratio: f32, weight: f32) -> Font {
    let size = TYPE_BODY_PT * ratio;
    Font(TextStyle::new(size, weight).lh(size * BODY_LINE_HEIGHT))
}

/// Weight 400: running text.
const WEIGHT_REGULAR: f32 = 400.0;
/// Weight 500: selected and secondary emphasis.
pub(crate) const WEIGHT_MEDIUM: f32 = 500.0;
/// Weight 600: names, titles, unread channels.
pub(crate) const WEIGHT_SEMIBOLD: f32 = 600.0;

/// Message text and channel names, at weight 400.
pub(crate) const TYPE_BODY: Font = type_scaled(1.0, WEIGHT_REGULAR);
/// `type.body` at weight 600: a primary button's label.
pub(crate) const TYPE_BODY_STRONG: Font = type_scaled(1.0, WEIGHT_SEMIBOLD);
/// `type.small` at weight 500: field labels, links under a form.
pub(crate) const TYPE_SMALL_MEDIUM: Font = type_scaled(12.0 / 13.0, WEIGHT_MEDIUM);
/// `type.small` at weight 400: a feedback line.
pub(crate) const TYPE_SMALL: Font = type_scaled(12.0 / 13.0, WEIGHT_REGULAR);
/// `type.caption`: sub-lines, a server's address, a field's hint.
pub(crate) const TYPE_CAPTION: Font = type_scaled(11.0 / 13.0, WEIGHT_REGULAR);
/// `type.title` at weight 600: a rail letter, the server card's name.
pub(crate) const TYPE_TITLE: Font = type_scaled(14.0 / 13.0, WEIGHT_SEMIBOLD);
/// `type.caption` at weight 600, its line exactly as tall as its size, so it
/// centres in a badge: a mention count.
pub(crate) const TYPE_BADGE: Font = {
    let size = TYPE_BODY_PT * 11.0 / 13.0;
    Font(TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size))
};
/// `type.caption` at weight 500: a group label in the channel list.
pub(crate) const TYPE_CAPTION_MEDIUM: Font = type_scaled(11.0 / 13.0, WEIGHT_MEDIUM);
/// `type.body` at weight 500: the selected channel, the account's name.
pub(crate) const TYPE_BODY_MEDIUM: Font = type_scaled(1.0, WEIGHT_MEDIUM);
/// `type.display` at weight 600: a screen's title, a large server initial.
pub(crate) const TYPE_DISPLAY: Font = type_scaled(18.0 / 13.0, WEIGHT_SEMIBOLD);

/// `radius.composer`, the message field's corner radius.
pub(crate) const RADIUS_COMPOSER: Radius = Radius(16.0);
/// `space.xs`.
pub(crate) const SPACE_XS: Space = Space(2.0);
/// `space.sm`.
pub(crate) const SPACE_SM: Space = Space(4.0);
/// `space.md`.
pub(crate) const SPACE_MD: Space = Space(8.0);
/// `space.lg`.
pub(crate) const SPACE_LG: Space = Space(12.0);
/// `space.xl`.
pub(crate) const SPACE_XL: Space = Space(16.0);
/// `space.xxl`.
pub(crate) const SPACE_XXL: Space = Space(20.0);
/// The composer pill's height (`docs/design.md` §5: pill heights scale with
/// spacing).
pub(crate) const COMPOSER_HEIGHT: Space = Space(44.0);
/// Send's square inside the composer pill (`docs/design.md` §5).
pub(crate) const SEND_BUTTON: Space = Space(30.0);
/// Send's corner radius: the mock's 10 px.
pub(crate) const RADIUS_SEND: Radius = Radius(10.0);
/// The most lines the composer grows to before it scrolls inside.
pub(crate) const COMPOSER_MAX_LINES: usize = 8;
/// The mark that takes the time's place on an unsent message, in points.
pub(crate) const SEND_STATE_ICON_PT: f32 = 14.0;
/// The width of a pill's outline in points.
pub(crate) const PILL_OUTLINE_PT: f32 = 1.0;
/// `radius.control`: rows, hover, small controls inside a field.
pub(crate) const RADIUS_CONTROL: Radius = Radius(8.0);
/// `radius.pill`, a header pill's corner radius.
pub(crate) const RADIUS_PILL: Radius = Radius(14.0);
/// The header pills' height.
pub(crate) const HEADER_PILL_HEIGHT: Space = Space(36.0);

mod background;
mod code_ink;
mod colour;
mod content;
mod density;
mod floors;
mod look;
mod reading;
mod replies;
mod sizes;
#[cfg(test)]
mod tests;

pub(crate) use background::{
    Accent, Background, BackgroundColour, CHROME_GRADIENT_DEGREES, ChromeFill,
};
pub(crate) use code_ink::CodeInk;
pub(crate) use colour::{Palette, Theme};
pub(crate) use content::*;
pub(crate) use density::*;
pub(crate) use look::{Glass, Look, LookSpec, Scale, Surfaces};
pub(crate) use reading::*;
pub(crate) use replies::*;
pub(crate) use sizes::*;
