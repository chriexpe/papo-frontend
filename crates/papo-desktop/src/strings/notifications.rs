//! Strings for Settings › Notifications, the server card, and OS notification
//! fallback bodies (`docs/design.md` §8 "D9 additions").

use super::{I18N, Key, PluralKey, catalog};

/// The Notifications settings page title.
pub(crate) fn settings_notifications() -> &'static str {
    text!("settings.notifications")
}

/// Nav summary for Notifications.
pub(crate) fn settings_notifications_summary(enabled: bool, tray_counter: bool) -> &'static str {
    match (enabled, tray_counter) {
        (true, true) => text!("settings.notifications_summary_on_tray"),
        (true, false) => text!("settings.notifications_summary_on"),
        (false, _) => text!("settings.notifications_summary_off"),
    }
}

/// "Message notifications".
pub(crate) fn settings_notify_messages() -> &'static str {
    text!("settings.notify_messages")
}

/// "Messages and mentions on this device".
pub(crate) fn settings_notify_messages_sub() -> &'static str {
    text!("settings.notify_messages_sub")
}

/// "Notify mentions".
pub(crate) fn settings_notify_mentions() -> &'static str {
    text!("settings.notify_mentions")
}

/// "Show the message in the notification".
pub(crate) fn settings_notify_preview() -> &'static str {
    text!("settings.notify_preview")
}

/// "Off, shows only who sent it".
pub(crate) fn settings_notify_preview_sub() -> &'static str {
    text!("settings.notify_preview_sub")
}

/// "Sound".
pub(crate) fn settings_notify_sound() -> &'static str {
    text!("settings.notify_sound")
}

/// "These follow your account on {server}."
pub(crate) fn settings_notify_sync_note(server: &str) -> String {
    template!("settings.notify_sync_note", "server" => server)
}

/// "Tray counter".
pub(crate) fn settings_tray_counter() -> &'static str {
    text!("settings.tray_counter")
}

/// "This device only".
pub(crate) fn settings_tray_counter_sub() -> &'static str {
    text!("settings.tray_counter_sub")
}

/// "Channel notifications".
pub(crate) fn settings_channel_notifications() -> &'static str {
    text!("settings.channel_notifications")
}

/// "Kept on each server's card".
pub(crate) fn settings_channel_notifications_sub() -> &'static str {
    text!("settings.channel_notifications_sub")
}

/// Spoken label for the server pill button that opens the server card.
pub(crate) fn server_card_open() -> &'static str {
    text!("server_card.open")
}

/// "{online} online · {n} people".
pub(crate) fn server_card_online_people(online: usize, total: usize) -> String {
    let people = plural!(
        "server_card.people",
        u64::try_from(total).unwrap_or(u64::MAX)
    );
    template!(
        "server_card.online_people",
        "online" => online,
        "people" => people
    )
}

/// "Copy address".
pub(crate) fn server_card_copy_address() -> &'static str {
    text!("server_card.copy_address")
}

/// "Copied".
pub(crate) fn server_card_copied() -> &'static str {
    text!("server_card.copied")
}

/// "NOTIFICATIONS" heading inside the server card.
pub(crate) fn server_card_notifications() -> &'static str {
    text!("server_card.notifications")
}

/// "All messages".
pub(crate) fn server_card_mode_all() -> &'static str {
    text!("server_card.mode_all")
}

/// "Mentions only".
pub(crate) fn server_card_mode_mentions() -> &'static str {
    text!("server_card.mode_mentions")
}

/// "Off".
pub(crate) fn server_card_mode_off() -> &'static str {
    text!("server_card.mode_off")
}

/// "All channels".
pub(crate) fn server_card_all_channels() -> &'static str {
    text!("server_card.all_channels")
}

/// "Mark all as read".
pub(crate) fn server_card_mark_all_read() -> &'static str {
    text!("server_card.mark_all_read")
}

/// Failure line when Mark all as read fails.
pub(crate) fn server_card_mark_all_read_failed() -> &'static str {
    text!("server_card.mark_all_read_failed")
}

/// Failure line when saving a channel notification mode fails.
pub(crate) fn server_card_mode_failed() -> &'static str {
    text!("server_card.mode_failed")
}

/// Fallback notification body when Show the message is off (channel).
pub(crate) fn notification_new_message() -> &'static str {
    text!("notification.new_message")
}

/// Fallback notification body when Show the message is off (direct message).
pub(crate) fn notification_direct_message() -> &'static str {
    text!("notification.direct_message")
}
