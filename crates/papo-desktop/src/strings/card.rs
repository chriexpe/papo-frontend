//! The strings of someone else's profile card (`docs/design.md` §10) and
//! the notices its actions give. The catalog entries are `card.*` and
//! their `notice.*`.

use super::{I18N, Key, catalog};

/// The card, for assistive technology: "Lia's profile".
pub(crate) fn card_label(name: &str) -> String {
    template!("card.label", "name" => name)
}

/// The roles section's heading.
pub(crate) fn card_roles() -> &'static str {
    text!("card.roles")
}

/// The about section's heading.
pub(crate) fn card_about() -> &'static str {
    text!("card.about")
}

/// "Member since 3 March 2025".
pub(crate) fn card_member_since(date: &str) -> String {
    template!("card.member_since", "date" => date)
}

/// Message: opens a direct conversation with them.
pub(crate) fn card_message() -> &'static str {
    text!("card.message")
}

pub(crate) fn card_copy_id() -> &'static str {
    text!("card.copy_id")
}

pub(crate) fn card_block() -> &'static str {
    text!("card.block")
}

pub(crate) fn card_unblock() -> &'static str {
    text!("card.unblock")
}

pub(crate) fn card_ban() -> &'static str {
    text!("card.ban")
}

/// "Ban Lia from Cyber Asilo?", asked inside the Ban row.
pub(crate) fn card_ban_ask(name: &str, server: &str) -> String {
    template!("card.ban_ask", "name" => name, "server" => server)
}

pub(crate) fn card_cancel() -> &'static str {
    text!("card.cancel")
}

pub(crate) fn notice_block_failed() -> &'static str {
    text!("notice.block_failed")
}

pub(crate) fn notice_unblock_failed() -> &'static str {
    text!("notice.unblock_failed")
}

pub(crate) fn notice_ban_failed() -> &'static str {
    text!("notice.ban_failed")
}

pub(crate) fn notice_dm_open_failed() -> &'static str {
    text!("notice.dm_open_failed")
}

/// The backend refuses a conversation with them: they blocked you, or the
/// server forbids it. Neither is said.
pub(crate) fn notice_dm_refused() -> &'static str {
    text!("notice.dm_refused")
}
