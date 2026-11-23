//! Papo's design tokens: colours (`colour`), type, radii and spacing.
//!
//! Every value is Papo's own design decision; nothing here comes from Rune's
//! default theme, and no view writes a colour or size by hand. Views own
//! layout; this module owns only the values they lay out with.
//!
//! Not built yet: the text-size, roundness and spacing multipliers (all at
//! ×1).
//!
//! The feedback colours (danger, warning, success) and the sizes of the server
//! and account screens come from the design project's "Onboarding and Sign In"
//! page, which `docs/design.md` §7 was settled from.

use rune::TextStyle;

/// `type.body` in points: message text and channel names. Every other text
/// size is a ratio of it.
pub(crate) const TYPE_BODY_PT: f32 = 14.0;
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
pub(crate) const WEIGHT_MEDIUM: f32 = 500.0;
/// Weight 600: names, titles, unread channels.
pub(crate) const WEIGHT_SEMIBOLD: f32 = 600.0;

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
/// `type.title` at weight 600: a rail letter, the server card's name.
pub(crate) const TYPE_TITLE: TextStyle = type_scaled(14.0 / 13.0, WEIGHT_SEMIBOLD);
/// `type.caption` at weight 600, its line exactly as tall as its size, so it
/// centres in a badge: a mention count.
pub(crate) const TYPE_BADGE: TextStyle = {
    let size = TYPE_BODY_PT * 11.0 / 13.0;
    TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size)
};
/// `type.caption` at weight 500: a group label in the channel list.
pub(crate) const TYPE_CAPTION_MEDIUM: TextStyle = type_scaled(11.0 / 13.0, WEIGHT_MEDIUM);
/// `type.body` at weight 500: the selected channel, the account's name.
pub(crate) const TYPE_BODY_MEDIUM: TextStyle = type_scaled(1.0, WEIGHT_MEDIUM);
/// `type.display` at weight 600: a screen's title, a large server initial.
pub(crate) const TYPE_DISPLAY: TextStyle = type_scaled(18.0 / 13.0, WEIGHT_SEMIBOLD);

/// `radius.composer`, the message field's corner radius in points.
pub(crate) const RADIUS_COMPOSER_PT: f32 = 16.0;
/// `space.xs` in points.
pub(crate) const SPACE_XS_PT: f32 = 2.0;
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
/// Send's square inside the composer pill, in points (`docs/design.md` §5).
pub(crate) const SEND_BUTTON_PT: f32 = 30.0;
/// Send's corner radius in points: the mock's 10 px.
pub(crate) const RADIUS_SEND_PT: f32 = 10.0;
/// The most lines the composer grows to before it scrolls inside.
pub(crate) const COMPOSER_MAX_LINES: usize = 8;
/// The mark that takes the time's place on an unsent message, in points.
pub(crate) const SEND_STATE_ICON_PT: f32 = 14.0;
/// The width of a pill's outline in points.
pub(crate) const PILL_OUTLINE_PT: f32 = 1.0;
/// `radius.control` in points: small controls inside a field.
pub(crate) const RADIUS_CONTROL_PT: f32 = 8.0;
/// `radius.pill`, a header pill's corner radius in points.
pub(crate) const RADIUS_PILL_PT: f32 = 14.0;
/// The header pills' height in points.
pub(crate) const HEADER_PILL_HEIGHT_PT: f32 = 36.0;

mod code_ink;
mod colour;
mod content;
mod density;
mod lightness_rule;
mod reading;
mod replies;
mod sizes;
#[cfg(test)]
mod tests;

pub(crate) use code_ink::CodeInk;
pub(crate) use colour::{Palette, Theme};
pub(crate) use content::*;
pub(crate) use density::*;
pub(crate) use reading::*;
pub(crate) use replies::*;
pub(crate) use sizes::*;
