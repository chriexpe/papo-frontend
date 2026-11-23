//! The strings of people (`docs/design.md` §10): presence, and how a person
//! is spoken with it, and direct messages. The catalog entries are
//! `presence.*` and `direct.*`.

use papo_sdk_ui::PresenceMark;

use super::{I18N, Key, catalog};

/// A presence's name: "Away".
pub(crate) fn presence(mark: PresenceMark) -> &'static str {
    match mark {
        PresenceMark::Available => text!("presence.available"),
        PresenceMark::Away => text!("presence.away"),
        PresenceMark::Busy => text!("presence.busy"),
        PresenceMark::Offline => text!("presence.offline"),
    }
}

/// A person spoken with their presence after the name: "Lia, Away".
pub(crate) fn person_with_presence(name: &str, mark: PresenceMark) -> String {
    template!("presence.named", "name" => name, "presence" => presence(mark))
}

/// The Direct pane's title, and the rail's Direct: "Direct messages".
pub(crate) fn direct_title() -> &'static str {
    text!("direct.title")
}

/// The list of direct conversations, for assistive technology.
pub(crate) fn direct_list() -> &'static str {
    text!("direct.list")
}

/// Direct with no conversation in it.
pub(crate) fn direct_empty() -> &'static str {
    text!("direct.empty")
}

/// A direct conversation's sub-line while the other person types.
pub(crate) fn direct_typing() -> &'static str {
    text!("direct.typing")
}

/// A row's ✕: "Close the conversation with Lia".
pub(crate) fn direct_close(name: &str) -> String {
    template!("direct.close", "name" => name)
}

/// What a row's keyboard can do beyond opening it.
pub(crate) fn direct_close_hint() -> &'static str {
    text!("direct.close_hint")
}

/// Closing a direct conversation did not happen.
pub(crate) fn notice_dm_close_failed() -> &'static str {
    text!("notice.dm_close_failed")
}

/// A direct conversation's title, spoken: "Lia, on Cyber Asilo".
pub(crate) fn direct_title_label(name: &str, server: &str) -> String {
    template!("direct.title_label", "name" => name, "server" => server)
}
