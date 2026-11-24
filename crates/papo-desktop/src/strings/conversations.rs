//! The Conversations page's strings (`docs/design.md` §8): its rows,
//! groups and the Message highlights sub-page. The catalog entries are
//! `settings.*`.

use super::{I18N, Key, catalog};

/// The Conversations page.
pub(crate) fn settings_conversations() -> &'static str {
    text!("settings.conversations")
}

/// The Conversations nav row's one line.
pub(crate) fn settings_conversations_summary() -> &'static str {
    text!("settings.conversations_summary")
}

/// Conversations' first group: "When writing".
pub(crate) fn settings_group_writing() -> &'static str {
    text!("settings.group_writing")
}

/// "Enter to send".
pub(crate) fn settings_enter_sends() -> &'static str {
    text!("settings.enter_sends")
}

/// Enter to send's summary.
pub(crate) fn settings_enter_sends_sub() -> &'static str {
    text!("settings.enter_sends_sub")
}

/// Conversations' second group: "In the conversation".
pub(crate) fn settings_group_conversation() -> &'static str {
    text!("settings.group_conversation")
}

/// "Show times".
pub(crate) fn settings_show_times() -> &'static str {
    text!("settings.show_times")
}

/// "Show avatars".
pub(crate) fn settings_show_avatars() -> &'static str {
    text!("settings.show_avatars")
}

/// "Open at the newest message".
pub(crate) fn settings_open_at_newest() -> &'static str {
    text!("settings.open_at_newest")
}

/// Open at the newest message's summary.
pub(crate) fn settings_open_at_newest_sub() -> &'static str {
    text!("settings.open_at_newest_sub")
}

/// "Unread hourglass".
pub(crate) fn settings_hourglass() -> &'static str {
    text!("settings.hourglass")
}

/// Unread hourglass's summary.
pub(crate) fn settings_hourglass_sub() -> &'static str {
    text!("settings.hourglass_sub")
}

/// "Description when opening a channel".
pub(crate) fn settings_channel_description() -> &'static str {
    text!("settings.channel_description")
}

/// Its summary: how long the description shows.
pub(crate) fn settings_channel_description_sub() -> &'static str {
    text!("settings.channel_description_sub")
}

/// The Message highlights row and sub-page.
pub(crate) fn settings_highlights() -> &'static str {
    text!("settings.highlights")
}

/// Message highlights' summary.
pub(crate) fn settings_highlights_sub() -> &'static str {
    text!("settings.highlights_sub")
}

/// Message highlights › Mentions.
pub(crate) fn settings_highlight_mentions() -> &'static str {
    text!("settings.highlight_mentions")
}

/// A hue swatch's spoken label: "Hue 75°".
pub(crate) fn settings_hue_spoken(hue: &str) -> String {
    template!("settings.hue_spoken", "hue" => hue)
}
