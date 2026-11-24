//! The account's pages' strings (`docs/design.md` §8): Profile and
//! Privacy and security, their groups, rows and the password form. The
//! catalog entries are `settings.*`; presence's names are `presence.*`.

use super::{I18N, Key, PluralKey, catalog};

/// The Profile page: the nav's subject tile.
pub(crate) fn settings_profile() -> &'static str {
    text!("settings.profile")
}

/// The subject tile's line.
pub(crate) fn settings_profile_summary() -> &'static str {
    text!("settings.profile_summary")
}

/// The Privacy and security page.
pub(crate) fn settings_privacy() -> &'static str {
    text!("settings.privacy")
}

/// "Nickname".
pub(crate) fn settings_nickname() -> &'static str {
    text!("settings.nickname")
}

/// "Status": the profile's status line.
pub(crate) fn settings_status() -> &'static str {
    text!("settings.status")
}

/// Its summary.
pub(crate) fn settings_status_sub() -> &'static str {
    text!("settings.status_sub")
}

/// Status's field, empty.
pub(crate) fn settings_status_placeholder() -> &'static str {
    text!("settings.status_placeholder")
}

/// "About you".
pub(crate) fn settings_about() -> &'static str {
    text!("settings.about")
}

/// "Presence".
pub(crate) fn settings_presence() -> &'static str {
    text!("settings.presence")
}

/// "Typing phrase".
pub(crate) fn settings_typing_phrase() -> &'static str {
    text!("settings.typing_phrase")
}

/// Typing phrase's field, empty: the phrase shown without one.
pub(crate) fn settings_typing_placeholder() -> &'static str {
    text!("settings.typing_placeholder")
}

/// A text row with no value.
pub(crate) fn settings_not_set() -> &'static str {
    text!("settings.not_set")
}

/// A text row's or the password form's Save.
pub(crate) fn settings_save() -> &'static str {
    text!("settings.save")
}

/// Save while it saves.
pub(crate) fn settings_saving() -> &'static str {
    text!("settings.saving")
}

/// Privacy and security's Privacy group.
pub(crate) fn settings_group_privacy() -> &'static str {
    text!("settings.group_privacy")
}

/// "Blocked people".
pub(crate) fn settings_blocked() -> &'static str {
    text!("settings.blocked")
}

/// Under the blocked list.
pub(crate) fn settings_blocked_note() -> &'static str {
    text!("settings.blocked_note")
}

/// Privacy and security's Sign-in group.
pub(crate) fn settings_group_sign_in() -> &'static str {
    text!("settings.group_sign_in")
}

/// "Change password".
pub(crate) fn settings_password() -> &'static str {
    text!("settings.password")
}

/// The form's first field.
pub(crate) fn settings_new_password() -> &'static str {
    text!("settings.new_password")
}

/// The form's second field.
pub(crate) fn settings_repeat_password() -> &'static str {
    text!("settings.repeat_password")
}

/// When the two fields differ.
pub(crate) fn settings_password_mismatch() -> &'static str {
    text!("settings.password_mismatch")
}

/// Under Change password once it is confirmed.
pub(crate) fn settings_password_changed() -> &'static str {
    text!("settings.password_changed")
}

/// Change password's checkbox.
pub(crate) fn settings_stay_signed_in() -> &'static str {
    text!("settings.stay_signed_in")
}

/// "Sign out".
pub(crate) fn settings_sign_out() -> &'static str {
    text!("settings.sign_out")
}

/// Sign out's red confirmation.
pub(crate) fn settings_sign_out_confirm() -> &'static str {
    text!("settings.sign_out_confirm")
}

/// The nav's group of the account's pages: "Account on Lumen".
pub(crate) fn settings_group_account(server: &str) -> String {
    template!("settings.group_account", "server" => server)
}

/// An account page's scope line.
pub(crate) fn settings_scope_account(server: &str) -> String {
    template!("settings.scope_account", "server" => server)
}

/// Profile's first group: "Profile on Lumen".
pub(crate) fn settings_group_profile(server: &str) -> String {
    template!("settings.group_profile", "server" => server)
}

/// Sign out's question, asked in the row.
pub(crate) fn settings_sign_out_question(server: &str) -> String {
    template!("settings.sign_out_question", "server" => server)
}

/// Change password's summary: "Only for @lia on Lumen".
pub(crate) fn settings_password_sub(handle: &str, server: &str) -> String {
    template!("settings.password_sub", "handle" => handle, "server" => server)
}

/// The password form's note.
pub(crate) fn settings_password_note(handle: &str, server: &str) -> String {
    template!("settings.password_note", "handle" => handle, "server" => server)
}

/// Sign out's summary: "Ends @lia's session on Lumen".
pub(crate) fn settings_sign_out_sub(handle: &str, server: &str) -> String {
    template!("settings.sign_out_sub", "handle" => handle, "server" => server)
}

/// A text row's count: "12/64".
pub(crate) fn settings_char_count(count: usize, most: usize) -> String {
    template!("settings.char_count", "count" => count, "most" => most)
}

/// The Privacy and security nav row's line: "2 blocked · password".
pub(crate) fn settings_privacy_summary(count: usize) -> String {
    plural!(
        "settings.privacy_summary",
        u64::try_from(count).unwrap_or(u64::MAX)
    )
}

/// Blocked people's summary: "2 people".
pub(crate) fn settings_blocked_sub(count: usize) -> String {
    plural!(
        "settings.blocked_sub",
        u64::try_from(count).unwrap_or(u64::MAX)
    )
}
