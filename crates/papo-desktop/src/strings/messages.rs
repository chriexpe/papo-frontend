//! The strings of a message's actions (`docs/design.md` §10): the hover
//! bar, the message and link menus, reactions, pins, and the notice that
//! says one of them did not happen. The catalog entries are `action.*`,
//! `menu.*`, `link_menu.*`, `reaction.*`, `reactions.*`, `pins.*` and
//! `notice.*`.

use super::{I18N, Key, catalog};

/// A reaction you added or removed did not happen.
pub(crate) fn notice_reaction_failed() -> &'static str {
    text!("notice.reaction_failed")
}

/// Pinning a message did not happen.
pub(crate) fn notice_pin_failed() -> &'static str {
    text!("notice.pin_failed")
}

/// Unpinning a message did not happen.
pub(crate) fn notice_unpin_failed() -> &'static str {
    text!("notice.unpin_failed")
}

/// A quick reaction's spoken name: "React with 👍".
pub(crate) fn reaction_react(emoji: &str) -> String {
    template!("reaction.react", "emoji" => emoji)
}

/// A quick reaction you already gave, which takes it back.
pub(crate) fn reaction_take_back(emoji: &str) -> String {
    template!("reaction.take_back", "emoji" => emoji)
}

/// The message menu's header: "Lia · Today at 14:02".
pub(crate) fn menu_header(author: &str, day: &str, time: &str) -> String {
    template!("menu.header", "author" => author, "day" => day, "time" => time)
}

/// Reply, in the message menu.
pub(crate) fn menu_reply() -> &'static str {
    text!("menu.reply")
}

/// Edit, in the message menu.
pub(crate) fn menu_edit() -> &'static str {
    text!("menu.edit")
}

/// Pin message.
pub(crate) fn menu_pin() -> &'static str {
    text!("menu.pin")
}

/// Unpin message.
pub(crate) fn menu_unpin() -> &'static str {
    text!("menu.unpin")
}

/// Copy text: the message as drawn.
pub(crate) fn menu_copy_text() -> &'static str {
    text!("menu.copy_text")
}

/// Copy as Markdown: the message's source.
pub(crate) fn menu_copy_markdown() -> &'static str {
    text!("menu.copy_markdown")
}

/// Copy ID.
pub(crate) fn menu_copy_id() -> &'static str {
    text!("menu.copy_id")
}

/// Delete message, last in the menu.
pub(crate) fn menu_delete() -> &'static str {
    text!("menu.delete")
}

/// Open link, in the message and link menus.
pub(crate) fn link_menu_open() -> &'static str {
    text!("link_menu.open")
}

/// Copy link, in the link menu.
pub(crate) fn link_menu_copy() -> &'static str {
    text!("link_menu.copy")
}

/// Reply's key.
pub(crate) fn shortcut_reply() -> &'static str {
    text!("shortcut.reply")
}

/// Edit's key.
pub(crate) fn shortcut_edit() -> &'static str {
    text!("shortcut.edit")
}

/// Pin's key.
pub(crate) fn shortcut_pin() -> &'static str {
    text!("shortcut.pin")
}

/// Copy text's key.
pub(crate) fn shortcut_copy() -> &'static str {
    text!("shortcut.copy")
}

/// Delete's key.
pub(crate) fn shortcut_delete() -> &'static str {
    text!("shortcut.delete")
}

/// Add reaction, on the hover bar.
pub(crate) fn action_add_reaction() -> &'static str {
    text!("action.add_reaction")
}

/// Reply, on the hover bar.
pub(crate) fn action_reply() -> &'static str {
    text!("action.reply")
}

/// Edit, on the hover bar.
pub(crate) fn action_edit() -> &'static str {
    text!("action.edit")
}

/// More, on the hover bar: the message menu.
pub(crate) fn action_more() -> &'static str {
    text!("action.more")
}

/// The hover bar's spoken name.
pub(crate) fn action_bar() -> &'static str {
    text!("action.bar")
}
