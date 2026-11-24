//! The General page's strings (`docs/design.md` §8): Language, the link
//! settings and the Trusted domains sub-page. The catalog entries are
//! `settings.*`; the languages' own names are the catalog's locales'.

use super::{I18N, Key, PluralKey, catalog};

/// The General page.
pub(crate) fn settings_general() -> &'static str {
    text!("settings.general")
}

/// "Language".
pub(crate) fn settings_language() -> &'static str {
    text!("settings.language")
}

/// Language's first choice: the system's.
pub(crate) fn settings_language_system() -> &'static str {
    text!("settings.language_system")
}

/// General's group of link settings.
pub(crate) fn settings_group_links() -> &'static str {
    text!("settings.group_links")
}

/// "Confirm external links".
pub(crate) fn settings_confirm_links() -> &'static str {
    text!("settings.confirm_links")
}

/// Its summary.
pub(crate) fn settings_confirm_links_sub() -> &'static str {
    text!("settings.confirm_links_sub")
}

/// The Trusted domains row and sub-page.
pub(crate) fn settings_trusted() -> &'static str {
    text!("settings.trusted")
}

/// The Trusted domains page with none.
pub(crate) fn settings_trusted_empty() -> &'static str {
    text!("settings.trusted_empty")
}

/// The domain field's example.
pub(crate) fn settings_trusted_placeholder() -> &'static str {
    text!("settings.trusted_placeholder")
}

/// The domain field's Add.
pub(crate) fn settings_trusted_add() -> &'static str {
    text!("settings.trusted_add")
}

/// A domain's Remove.
pub(crate) fn settings_trusted_remove() -> &'static str {
    text!("settings.trusted_remove")
}

/// Under the field when what was typed names no site.
pub(crate) fn settings_trusted_invalid() -> &'static str {
    text!("settings.trusted_invalid")
}

/// The General nav row's one line: "System · links".
pub(crate) fn settings_general_summary(language: &str) -> String {
    template!("settings.general_summary", "language" => language)
}

/// Trusted domains' summary: "3 sites open directly".
pub(crate) fn settings_trusted_sub(count: usize) -> String {
    plural!(
        "settings.trusted_sub",
        u64::try_from(count).unwrap_or(u64::MAX)
    )
}

/// A domain's Remove, spoken: "Remove example.org".
pub(crate) fn settings_trusted_remove_spoken(host: &str) -> String {
    template!("settings.trusted_remove_spoken", "host" => host)
}
