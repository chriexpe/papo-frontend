//! The keyboard shortcuts sheet's strings (`docs/design.md` §12). The
//! catalog entries are `shortcuts.*`.

use super::{I18N, Key, catalog};

/// A group: keys that work from any focus.
pub(crate) fn shortcuts_anywhere() -> &'static str {
    text!("shortcuts.anywhere")
}

/// Ctrl+K's row.
pub(crate) fn shortcuts_jump() -> &'static str {
    text!("shortcuts.jump")
}

/// Ctrl+1–9's row.
pub(crate) fn shortcuts_server_number() -> &'static str {
    text!("shortcuts.server_number")
}

/// The tab keys' row.
pub(crate) fn shortcuts_server_step() -> &'static str {
    text!("shortcuts.server_step")
}

/// Under the tab keys' row.
pub(crate) fn shortcuts_server_step_note() -> &'static str {
    text!("shortcuts.server_step_note")
}

/// Alt+↑/↓'s row.
pub(crate) fn shortcuts_channel_step() -> &'static str {
    text!("shortcuts.channel_step")
}

/// Alt+⇧↑/↓'s row.
pub(crate) fn shortcuts_unread_step() -> &'static str {
    text!("shortcuts.unread_step")
}

/// Alt+←/→'s row.
pub(crate) fn shortcuts_history() -> &'static str {
    text!("shortcuts.history")
}

/// The sheet's title, and its own row.
pub(crate) fn shortcuts_title() -> &'static str {
    text!("shortcuts.title")
}

/// F6's row.
pub(crate) fn shortcuts_region() -> &'static str {
    text!("shortcuts.region")
}

/// A group: the header's panels.
pub(crate) fn shortcuts_panels() -> &'static str {
    text!("shortcuts.panels")
}

/// Ctrl+U's row.
pub(crate) fn shortcuts_dm_profile() -> &'static str {
    text!("shortcuts.dm_profile")
}

/// Escape's row in Panels.
pub(crate) fn shortcuts_close_top() -> &'static str {
    text!("shortcuts.close_top")
}

/// A group: the text pill.
pub(crate) fn shortcuts_composer() -> &'static str {
    text!("shortcuts.composer")
}

/// Enter's row.
pub(crate) fn shortcuts_send() -> &'static str {
    text!("shortcuts.send")
}

/// Under Send.
pub(crate) fn shortcuts_send_note() -> &'static str {
    text!("shortcuts.send_note")
}

/// Ctrl+Enter's row.
pub(crate) fn shortcuts_send_always() -> &'static str {
    text!("shortcuts.send_always")
}

/// Ctrl+↑'s row.
pub(crate) fn shortcuts_into_conversation() -> &'static str {
    text!("shortcuts.into_conversation")
}

/// Under Ctrl+↑'s row.
pub(crate) fn shortcuts_into_conversation_note() -> &'static str {
    text!("shortcuts.into_conversation_note")
}

/// ↑'s row.
pub(crate) fn shortcuts_edit_newest() -> &'static str {
    text!("shortcuts.edit_newest")
}

/// Under ↑'s row.
pub(crate) fn shortcuts_field_empty() -> &'static str {
    text!("shortcuts.field_empty")
}

/// Escape's row in Composer.
pub(crate) fn shortcuts_cancel_mode() -> &'static str {
    text!("shortcuts.cancel_mode")
}

/// Ctrl+B/I's row.
pub(crate) fn shortcuts_bold_italic() -> &'static str {
    text!("shortcuts.bold_italic")
}

/// Ctrl+Z's row.
pub(crate) fn shortcuts_undo() -> &'static str {
    text!("shortcuts.undo")
}

/// A group: keys on a focused message.
pub(crate) fn shortcuts_message() -> &'static str {
    text!("shortcuts.message")
}

/// The arrows' row on a message.
pub(crate) fn shortcuts_move() -> &'static str {
    text!("shortcuts.move")
}

/// Enter's row on a message.
pub(crate) fn shortcuts_open_menu() -> &'static str {
    text!("shortcuts.open_menu")
}

/// R, E, P's row.
pub(crate) fn shortcuts_reply_edit_pin() -> &'static str {
    text!("shortcuts.reply_edit_pin")
}

/// 1 2 3's row.
pub(crate) fn shortcuts_react_top() -> &'static str {
    text!("shortcuts.react_top")
}

/// +'s row.
pub(crate) fn shortcuts_add_reaction() -> &'static str {
    text!("shortcuts.add_reaction")
}

/// Ctrl+C/A's row.
pub(crate) fn shortcuts_copy_select() -> &'static str {
    text!("shortcuts.copy_select")
}

/// Delete's row.
pub(crate) fn shortcuts_delete() -> &'static str {
    text!("shortcuts.delete")
}

/// Escape's row on a message.
pub(crate) fn shortcuts_back_to_composer() -> &'static str {
    text!("shortcuts.back_to_composer")
}

/// A group: lists and menus.
pub(crate) fn shortcuts_menus() -> &'static str {
    text!("shortcuts.menus")
}

/// The arrows' row in lists.
pub(crate) fn shortcuts_list_move() -> &'static str {
    text!("shortcuts.list_move")
}

/// a–z's row.
pub(crate) fn shortcuts_letter() -> &'static str {
    text!("shortcuts.letter")
}

/// Enter's row in lists.
pub(crate) fn shortcuts_run() -> &'static str {
    text!("shortcuts.run")
}

/// Escape's row in lists.
pub(crate) fn shortcuts_close_back() -> &'static str {
    text!("shortcuts.close_back")
}

/// A group: the window.
pub(crate) fn shortcuts_window() -> &'static str {
    text!("shortcuts.window")
}

/// Ctrl+W's row.
pub(crate) fn shortcuts_close_window() -> &'static str {
    text!("shortcuts.close_window")
}

/// Ctrl+Q's row.
pub(crate) fn shortcuts_quit() -> &'static str {
    text!("shortcuts.quit")
}

/// F11's row.
pub(crate) fn shortcuts_full_screen() -> &'static str {
    text!("shortcuts.full_screen")
}

/// The Papo keys' column.
pub(crate) fn shortcuts_papo() -> &'static str {
    text!("shortcuts.papo")
}

/// The system keys' column.
pub(crate) fn shortcuts_system() -> &'static str {
    text!("shortcuts.system")
}

/// A column with no key, spoken.
pub(crate) fn shortcuts_none() -> &'static str {
    text!("shortcuts.none")
}

/// Between alternative keys, spoken.
pub(crate) fn shortcuts_or() -> &'static str {
    text!("shortcuts.or")
}
