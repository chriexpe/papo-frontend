//! The account's pages (`docs/design.md` §8): Profile (Nickname, Status,
//! About you; Presence, Typing phrase), and Privacy and security (Blocked
//! people with Unblock, Change password, Sign out). Photo and Banner come
//! with D7's image editor.
//!
//! Owns these pages' rows; the profile is the SDK's (`own_profile`), what
//! the rows do the shell's (`shell/settings_account`).

use papo_sdk_ui::PresenceMark;

use super::{Context, Group, Kind, Row};
use crate::own_profile::OwnProfile;
use crate::server_list::BlockedPerson;
use crate::settings::{PageId, Presence, RowId};
use crate::strings;

/// The backend's longest nickname, status line, about and typing phrase,
/// in characters (`services/users.go`; not served).
pub(crate) const NICKNAME_MOST: usize = 32;
pub(crate) const STATUS_MOST: usize = 64;
pub(crate) const ABOUT_MOST: usize = 512;
pub(crate) const TYPING_MOST: usize = 64;

/// What the account's pages are drawn from: the panel's account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AccountFacts {
    /// Its server's name.
    pub(crate) server: String,
    /// Its username, without the @.
    pub(crate) handle: String,
    pub(crate) own: OwnProfile,
    /// Its live presence, once the connection said.
    pub(crate) presence: Option<PresenceMark>,
    pub(crate) blocked: Vec<BlockedPerson>,
    /// Its password was changed while the panel is open.
    pub(crate) password_changed: bool,
    /// The destructive row asking its question, if one is.
    pub(crate) asking: Option<RowId>,
}

/// A text row: its value or what shows without one, and its longest.
fn text_row(
    id: RowId,
    title: &str,
    sub: Option<&str>,
    value: Option<&String>,
    placeholder: &str,
    most: usize,
    own: &OwnProfile,
) -> Row {
    Row {
        id,
        title: title.to_owned(),
        sub: sub.map(str::to_owned),
        kind: Kind::Text {
            value: value.cloned(),
            placeholder: placeholder.to_owned(),
            most,
            editable: own.known,
        },
    }
}

pub(super) fn profile(facts: &AccountFacts) -> Vec<Group> {
    let own = &facts.own;
    let not_set = strings::settings_not_set();
    vec![
        Group {
            title: Some(strings::settings_group_profile(&facts.server)),
            rows: vec![
                text_row(
                    RowId::Nickname,
                    strings::settings_nickname(),
                    None,
                    own.nickname.as_ref(),
                    not_set,
                    NICKNAME_MOST,
                    own,
                ),
                text_row(
                    RowId::Status,
                    strings::settings_status(),
                    Some(strings::settings_status_sub()),
                    own.status.as_ref(),
                    strings::settings_status_placeholder(),
                    STATUS_MOST,
                    own,
                ),
                text_row(
                    RowId::About,
                    strings::settings_about(),
                    None,
                    own.about.as_ref(),
                    not_set,
                    ABOUT_MOST,
                    own,
                ),
            ],
        },
        // No group title: it would repeat its first row's (§8).
        Group {
            title: None,
            rows: vec![
                Row {
                    id: RowId::Presence,
                    title: strings::settings_presence().to_owned(),
                    sub: None,
                    kind: Kind::Choice(presence_name(chosen_presence(facts.presence)).to_owned()),
                },
                text_row(
                    RowId::TypingPhrase,
                    strings::settings_typing_phrase(),
                    None,
                    own.typing.as_ref(),
                    strings::settings_typing_placeholder(),
                    TYPING_MOST,
                    own,
                ),
            ],
        },
    ]
}

pub(super) fn privacy(facts: &AccountFacts) -> Vec<Group> {
    let password_sub = if facts.password_changed {
        strings::settings_password_changed().to_owned()
    } else {
        strings::settings_password_sub(&facts.handle, &facts.server)
    };
    vec![
        Group {
            title: Some(strings::settings_group_privacy().to_owned()),
            rows: vec![Row {
                id: RowId::Blocked,
                title: strings::settings_blocked().to_owned(),
                sub: Some(strings::settings_blocked_sub(facts.blocked.len())),
                kind: Kind::SubPage(PageId::Blocked, String::new()),
            }],
        },
        Group {
            title: Some(strings::settings_group_sign_in().to_owned()),
            rows: vec![
                Row {
                    id: RowId::Password,
                    title: strings::settings_password().to_owned(),
                    sub: Some(password_sub),
                    kind: Kind::SubPage(PageId::Password, String::new()),
                },
                // Destructive rows come last (§8).
                Row {
                    id: RowId::SignOut,
                    title: strings::settings_sign_out().to_owned(),
                    sub: Some(strings::settings_sign_out_sub(&facts.handle, &facts.server)),
                    kind: Kind::Destructive {
                        question: strings::settings_sign_out_question(&facts.server),
                        confirm: strings::settings_sign_out_confirm().to_owned(),
                        asking: facts.asking == Some(RowId::SignOut),
                        busy: false,
                    },
                },
            ],
        },
    ]
}

pub(super) fn blocked(facts: &AccountFacts) -> Vec<Group> {
    let people: Vec<Row> = facts
        .blocked
        .iter()
        .enumerate()
        .map(|(index, person)| {
            let name = person
                .name
                .clone()
                .unwrap_or_else(|| strings::someone().to_owned());
            Row {
                id: RowId::BlockedPerson(index),
                title: name.clone(),
                sub: None,
                kind: Kind::Unblock(name),
            }
        })
        .collect();
    let list = if people.is_empty() {
        vec![Row {
            id: RowId::BlockedPerson(0),
            title: strings::direct_blocked_empty().to_owned(),
            sub: None,
            kind: Kind::Note,
        }]
    } else {
        people
    };
    vec![
        Group {
            title: None,
            rows: list,
        },
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::Blocked,
                title: strings::settings_blocked_note().to_owned(),
                sub: None,
                kind: Kind::Note,
            }],
        },
    ]
}

pub(super) fn password(facts: &AccountFacts) -> Vec<Group> {
    vec![Group {
        title: None,
        rows: vec![Row {
            id: RowId::PasswordForm,
            title: strings::settings_password().to_owned(),
            sub: Some(strings::settings_password_note(
                &facts.handle,
                &facts.server,
            )),
            kind: Kind::PasswordForm,
        }],
    }]
}

/// The presence the account chose, as its live presence shows it.
pub(crate) fn chosen_presence(mark: Option<PresenceMark>) -> Presence {
    match mark {
        Some(PresenceMark::Away) => Presence::Away,
        Some(PresenceMark::Busy) => Presence::Busy,
        _ => Presence::Available,
    }
}

pub(crate) fn presence_name(presence: Presence) -> &'static str {
    match presence {
        Presence::Available => strings::presence(PresenceMark::Available),
        Presence::Away => strings::presence(PresenceMark::Away),
        Presence::Busy => strings::presence(PresenceMark::Busy),
    }
}

/// The subject tile's line, and the Profile page's search words.
pub(crate) fn profile_summary() -> &'static str {
    strings::settings_profile_summary()
}

/// The Privacy and security nav row's line: "2 blocked · password".
pub(crate) fn privacy_summary(facts: &AccountFacts) -> String {
    strings::settings_privacy_summary(facts.blocked.len())
}

/// The account's pages need an account; without one they have no rows.
pub(super) fn of(page: PageId, cx: &Context<'_>) -> Vec<Group> {
    let Some(facts) = cx.account else {
        return Vec::new();
    };
    match page {
        PageId::Profile => profile(facts),
        PageId::Privacy => privacy(facts),
        PageId::Blocked => blocked(facts),
        PageId::Password => password(facts),
        _ => Vec::new(),
    }
}
