//! The strings of people (`docs/design.md` §10): presence and how a person
//! is spoken with it, direct messages, and who is typing. The catalog
//! entries are
//! `presence.*`, `direct.*` and `typing.*`.

use std::ops::Range;

use papo_sdk_ui::PresenceMark;

use super::{I18N, Key, catalog};

/// A presence's name: "Away".
pub(crate) fn presence(mark: PresenceMark) -> &'static str {
    match mark {
        PresenceMark::Away => text!("presence.away"),
        PresenceMark::Busy => text!("presence.busy"),
        PresenceMark::Offline => text!("presence.offline"),
        // Available, and a mark this client does not know yet: connected.
        _ => text!("presence.available"),
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

/// Who is typing (`docs/design.md` §10, Typing line), and where each name
/// sits in it (byte ranges, drawn at 600): "Lia is typing…", "Yuki is
/// thinking very hard…" (a personal phrase, one typer only), "Lia and Yuki
/// are typing…", "Lia, Yuki and Caio are typing…", then "Several people
/// are typing…". Empty with nobody.
pub(crate) fn typing_line(names: &[&str], phrase: Option<&str>) -> (String, Vec<Range<usize>>) {
    // Each name, and the phrase, goes in as a private-use marker, so it is
    // found again in any translation's order without matching the words
    // around it; nothing from the server is scanned for markers.
    const MARKERS: [&str; 4] = ["\u{E000}", "\u{E001}", "\u{E002}", "\u{E003}"];
    const PHRASE: usize = 3;
    let line = match (names, phrase) {
        ([], _) => return (String::new(), Vec::new()),
        ([_], Some(_)) => {
            template!("typing.one_phrase", "name" => MARKERS[0], "phrase" => MARKERS[PHRASE])
        }
        ([_], None) => template!("typing.one", "name" => MARKERS[0]),
        ([_, _], _) => template!("typing.two", "first" => MARKERS[0], "second" => MARKERS[1]),
        ([_, _, _], _) => template!(
            "typing.three",
            "first" => MARKERS[0],
            "second" => MARKERS[1],
            "third" => MARKERS[2]
        ),
        _ => return (text!("typing.many").to_owned(), Vec::new()),
    };
    let mut filled = String::with_capacity(line.len());
    let mut ranges = Vec::new();
    let mut rest = line.as_str();
    while let Some((at, index)) = MARKERS
        .iter()
        .enumerate()
        .filter_map(|(index, marker)| rest.find(marker).map(|at| (at, index)))
        .min()
    {
        filled.push_str(&rest[..at]);
        if index == PHRASE {
            filled.push_str(phrase.unwrap_or_default());
        } else {
            let start = filled.len();
            filled.push_str(names.get(index).copied().unwrap_or_default());
            ranges.push(start..filled.len());
        }
        rest = &rest[at + MARKERS[index].len()..];
    }
    filled.push_str(rest);
    (filled, ranges)
}

/// A DM's state pill when this account blocks them: "You blocked Lia.".
pub(crate) fn direct_you_blocked(name: &str) -> String {
    template!("direct.you_blocked", "name" => name)
}

/// A DM's state pill when the backend refuses it, never saying who blocked
/// whom: "You can't message Lia.".
pub(crate) fn direct_cant_message(name: &str) -> String {
    template!("direct.cant_message", "name" => name)
}

/// A DM's profile pane, and the button that shows it.
pub(crate) fn direct_profile() -> &'static str {
    text!("direct.profile")
}

/// The profile pane's shortcut, as its hint shows it.
pub(crate) fn shortcut_profile() -> &'static str {
    text!("shortcut.profile")
}

/// The profile pane's roles heading: "Roles in Cyber Asilo".
pub(crate) fn direct_roles_in(server: &str) -> String {
    template!("direct.roles_in", "server" => server)
}

/// Direct's pill while it shows the people this account blocks.
pub(crate) fn direct_blocked_title() -> &'static str {
    text!("direct.blocked_title")
}

/// The pill's button that shows or hides them.
pub(crate) fn direct_blocked_button() -> &'static str {
    text!("direct.blocked_button")
}

/// The list of blocked people, for assistive technology.
pub(crate) fn direct_blocked_list() -> &'static str {
    text!("direct.blocked_list")
}

/// No one blocked.
pub(crate) fn direct_blocked_empty() -> &'static str {
    text!("direct.blocked_empty")
}

/// A blocked person's Unblock: "Unblock Lia".
pub(crate) fn direct_unblock(name: &str) -> String {
    template!("direct.unblock", "name" => name)
}

/// Direct's New message button, and the picker's name.
pub(crate) fn direct_new() -> &'static str {
    text!("direct.new")
}

/// The New message picker's field, empty.
pub(crate) fn direct_find() -> &'static str {
    text!("direct.find")
}

/// The New message picker when the field finds no one.
pub(crate) fn direct_find_none() -> &'static str {
    text!("direct.find_none")
}

/// The New message picker's list, spoken.
pub(crate) fn direct_find_list() -> &'static str {
    text!("direct.find_list")
}
