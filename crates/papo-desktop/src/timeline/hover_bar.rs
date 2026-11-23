//! A message's hover bar (`docs/design.md` §10, MessageActions): a small
//! pill at the row's top-right, 12 px in and centred on the row's top edge,
//! shown while the pointer is on the row or the row has the keyboard, at
//! once and gone at once. Left to right: your three most used emoji, a
//! divider, Add reaction, Reply, Edit (your own messages), More.
//! Add reaction stretches the bar to the left into your eight most used,
//! its own buttons kept where they were. The bar's buttons are not Tab stops: a focused
//! message's keys reach them (`message_keys`). It never changes the row's
//! height: it floats over the distance above it.
//!
//! What the bar offers is `Bar`, read with the row's parts; presses go to
//! the host.

use papo_sdk::Uuid;
use rune::{Anim, El, Icon, el, icon, row, text};

use super::TimelineKey;
use super::view::TimelineHost;
use crate::design::{
    PILL_BUTTON_PT, PILL_ICON_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, SPACE_LG_PT,
    SPACE_SM_PT, SPACE_XS_PT, TYPE_BODY,
};
use crate::strings;

/// The bar's height, in points (`size.hover-bar`).
pub(super) const BAR_HEIGHT_PT: f32 = 32.0;
/// `radius.card`.
const RADIUS_CARD_PT: f32 = 10.0;
/// The row's hover curves, which its bar follows: it shows and goes at
/// once (Christian, 2026-11-23: a fade either way is in the way when the
/// pointer moves down the rows).
pub(super) fn row_motion() -> (Anim, Anim) {
    (Anim::INSTANT, Anim::INSTANT)
}
/// How many quick reactions lead the folded bar.
pub(crate) const BAR_REACTIONS: usize = 3;

/// What message `id`'s bar offers.
#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct Bar {
    /// Quick reactions shown, most used first, and whether each is yours:
    /// three folded, eight stretched. Empty when you may not react.
    pub(crate) emoji: Vec<(String, bool)>,
    /// Add reaction stretched the bar: the emoji ← and → are on.
    pub(crate) stretched: Option<usize>,
    pub(crate) reply: bool,
    pub(crate) edit: bool,
}

/// What a press on the bar asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BarPress {
    /// Toggle your reaction `emoji`.
    React {
        emoji: String,
    },
    /// Add reaction: the bar stretches into the strip, or folds back.
    AddReaction,
    Reply,
    Edit,
    /// More: the message menu, grown from the bar.
    More,
}

/// The element key of message `id`'s More, which its menu grows from.
pub(crate) fn more_key(id: Uuid) -> String {
    format!("more-{id}")
}

/// Message `id`'s bar, pinned over the row's top-right and seen while its
/// hover group (the row) is lit.
pub(super) fn bar<S: TimelineHost>(key: TimelineKey, id: Uuid, bar: &Bar, palette: &Palette) -> El {
    let mut buttons = emoji_buttons::<S>(key, id, bar, palette);
    if !buttons.is_empty() {
        buttons.push(
            el().w(PILL_OUTLINE_PT)
                .h(PILL_BUTTON_PT - 2.0 * SPACE_SM_PT)
                .mx(SPACE_XS_PT)
                .fill(palette.line_subtle)
                .a11y_hidden(),
        );
        buttons.push(icon_button::<S>(
            key,
            id,
            "bar-add-reaction",
            &crate::icons::SMILE_PLUS,
            strings::action_add_reaction(),
            None,
            BarPress::AddReaction,
            palette,
        ));
    }
    if bar.reply {
        buttons.push(icon_button::<S>(
            key,
            id,
            "bar-reply",
            &crate::icons::REPLY,
            strings::action_reply(),
            Some(strings::shortcut_reply()),
            BarPress::Reply,
            palette,
        ));
    }
    if bar.edit {
        buttons.push(icon_button::<S>(
            key,
            id,
            "bar-edit",
            &crate::icons::EDIT,
            strings::action_edit(),
            Some(strings::shortcut_edit()),
            BarPress::Edit,
            palette,
        ));
    }
    buttons.push(
        icon_button::<S>(
            key,
            id,
            "bar-more",
            &crate::icons::MORE,
            strings::action_more(),
            None,
            BarPress::More,
            palette,
        )
        .key(more_key(id)),
    );
    row()
        .key("bar")
        .h(BAR_HEIGHT_PT)
        .px(SPACE_XS_PT)
        .gap(SPACE_XS_PT)
        .center_y()
        .radius(RADIUS_CARD_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("toolbar")
        .label(strings::action_bar())
        // The bar's whole surface takes the pointer, not only its buttons
        // (its padding, the divider, the gaps, the corners): over the row
        // above, the bar stays its row's, and a press there falls through
        // to nothing.
        .on_background_press(|_: &mut S| {})
        .pin(None, Some(-BAR_HEIGHT_PT / 2.0), Some(SPACE_LG_PT), None)
        .group_opacity(0.0, 1.0)
        .children(buttons)
}

/// The bar's quick reactions: yours wear your chip's colours, and the one
/// ← and → are on while it is stretched wears the focus ring.
fn emoji_buttons<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    bar: &Bar,
    palette: &Palette,
) -> Vec<El> {
    bar.emoji
        .iter()
        .enumerate()
        .map(|(index, (emoji, mine))| {
            let press = BarPress::React {
                emoji: emoji.clone(),
            };
            let label = if *mine {
                strings::reaction_take_back(emoji)
            } else {
                strings::reaction_react(emoji)
            };
            let mut cell = button::<S>(key, id, format!("bar-emoji-{index}"), press, palette)
                .label(label)
                .child(text(emoji.clone()).font(TYPE_BODY));
            if *mine {
                cell = cell
                    .fill(palette.accent_soft)
                    .border(PILL_OUTLINE_PT, palette.accent);
            }
            if bar.stretched == Some(index) {
                cell = cell.border(PILL_OUTLINE_PT * 2.0, palette.focus_ring);
            }
            cell
        })
        .collect()
}

/// One of the bar's buttons: pressed by the pointer, never a Tab stop.
fn button<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    name: String,
    press: BarPress,
    palette: &Palette,
) -> El {
    row()
        .key(name)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .hover(palette.surface_hover)
        .on_press_tx(move |host: &mut S, tx| host.bar_pressed(key, id, press.clone(), tx))
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call per button, each naming what it is"
)]
fn icon_button<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    name: &str,
    shape: &'static Icon,
    label: &'static str,
    shortcut: Option<&'static str>,
    press: BarPress,
    palette: &Palette,
) -> El {
    let button = button::<S>(key, id, name.to_owned(), press, palette)
        .label(label)
        .help(label)
        .child(icon(shape, PILL_ICON_PT).color(palette.text_secondary));
    match shortcut {
        Some(shortcut) => button.help_keys([shortcut]),
        None => button,
    }
}
