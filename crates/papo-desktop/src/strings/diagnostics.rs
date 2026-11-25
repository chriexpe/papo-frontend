//! The Diagnostics page's strings (`docs/design.md` §8): the connection,
//! the media cache and the version, and the sizes and counts they show, in
//! the locale's number format. The catalog entries are `settings.*` and
//! `size.*`.

use super::{I18N, Key, catalog};

/// The Diagnostics page.
pub(crate) fn settings_diagnostics() -> &'static str {
    text!("settings.diagnostics")
}

/// The Diagnostics nav row's one line.
pub(crate) fn settings_diagnostics_summary() -> &'static str {
    text!("settings.diagnostics_summary")
}

/// Diagnostics' Connection group.
pub(crate) fn settings_group_connection() -> &'static str {
    text!("settings.group_connection")
}

/// "Connected".
pub(crate) fn settings_connected() -> &'static str {
    text!("settings.connected")
}

/// While the connection opens.
pub(crate) fn settings_connecting() -> &'static str {
    text!("settings.connecting")
}

/// While there is no connection.
pub(crate) fn settings_offline() -> &'static str {
    text!("settings.offline")
}

/// "Reconnect".
pub(crate) fn settings_reconnect() -> &'static str {
    text!("settings.reconnect")
}

/// Diagnostics' Cache group.
pub(crate) fn settings_group_cache() -> &'static str {
    text!("settings.group_cache")
}

/// "Media cache".
pub(crate) fn settings_cache() -> &'static str {
    text!("settings.cache")
}

/// "Clear media cache".
pub(crate) fn settings_clear_cache() -> &'static str {
    text!("settings.clear_cache")
}

/// Its summary.
pub(crate) fn settings_clear_cache_sub() -> &'static str {
    text!("settings.clear_cache_sub")
}

/// Its question, asked in the row.
pub(crate) fn settings_clear_cache_question() -> &'static str {
    text!("settings.clear_cache_question")
}

/// Its red confirmation.
pub(crate) fn settings_clear_cache_confirm() -> &'static str {
    text!("settings.clear_cache_confirm")
}

/// Diagnostics' About group.
pub(crate) fn settings_group_about() -> &'static str {
    text!("settings.group_about")
}

/// "Papo for desktop", beside the version.
pub(crate) fn settings_version() -> &'static str {
    text!("settings.version")
}

/// Connected's summary: "42 ms to Lumen".
pub(crate) fn settings_connected_sub(ms: &str, server: &str) -> String {
    template!("settings.connected_sub", "ms" => ms, "server" => server)
}

/// Connected's summary before the first round trip: "Connected to Lumen".
pub(crate) fn settings_connected_to(server: &str) -> String {
    template!("settings.connected_to", "server" => server)
}

/// Media cache's summary: "12.4 MB used".
pub(crate) fn settings_cache_sub(size: &str) -> String {
    template!("settings.cache_sub", "size" => size)
}

/// Under Clear media cache when the clear failed.
pub(crate) fn settings_clear_cache_failed() -> &'static str {
    text!("settings.clear_cache_failed")
}

/// A whole number in the locale's format.
pub(crate) fn integer(value: i64) -> String {
    I18N.integer(value)
}

/// `bytes` in KB, MB or GB, one decimal, in the locale's format. A KB is
/// 1024 bytes (Christian, 2026-11-24), so the SDK's 100 MiB limit reads
/// "100 MB".
pub(crate) fn size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    #[expect(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
    let bytes = bytes as f64;
    if bytes < KB * KB {
        let value = I18N.decimal(bytes / KB, 1);
        template!("size.kb", "value" => value)
    } else if bytes < KB * KB * KB {
        let value = I18N.decimal(bytes / (KB * KB), 1);
        template!("size.mb", "value" => value)
    } else {
        let value = I18N.decimal(bytes / (KB * KB * KB), 1);
        template!("size.gb", "value" => value)
    }
}
