//! The signed-in account's own profile, as Settings › Profile shows and
//! edits it (`docs/design.md` §8): nickname, status line, about, typing
//! phrase, roles and since when, read from the account's resource
//! snapshot.
//!
//! The backend replaces nickname, status line and about together, so the
//! page edits a text row only once the profile has been read back
//! (`known`); the SDK merges each change over what the server holds
//! (`change_profile`). Owns only the reading; the rows are `settings`',
//! saving the shell's (`shell/settings_profile`).

use papo_sdk::{ResourceSnapshot, Uuid};
use papo_sdk_ui::{person, requested_profile};

/// What the account's resources say about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnProfile {
    pub(crate) nickname: Option<String>,
    pub(crate) username: Option<String>,
    /// The profile's saved status line.
    pub(crate) status: Option<String>,
    pub(crate) about: Option<String>,
    pub(crate) typing: Option<String>,
    pub(crate) roles: Vec<String>,
    /// When the account was created, as the backend writes it.
    pub(crate) since: Option<String>,
    /// The profile has been read back, so its text rows can be edited.
    pub(crate) known: bool,
}

impl OwnProfile {
    /// `me`'s profile in `snapshot`.
    pub(crate) fn of(snapshot: &ResourceSnapshot, me: Uuid) -> OwnProfile {
        let member = person(snapshot, me);
        let profile = requested_profile(snapshot, me);
        let text = |field: Option<String>| field.filter(|text| !text.trim().is_empty());
        OwnProfile {
            nickname: text(member.nickname.clone()),
            username: member.username.clone(),
            roles: member
                .roles
                .iter()
                .filter_map(|role| role.name.clone())
                .collect(),
            known: profile.is_some(),
            status: text(
                profile
                    .as_ref()
                    .and_then(|profile| profile.status_message.clone()),
            ),
            about: text(
                profile
                    .as_ref()
                    .and_then(|profile| profile.description.clone()),
            ),
            typing: text(
                profile
                    .as_ref()
                    .and_then(|profile| profile.typing_phrase.clone()),
            ),
            since: profile.and_then(|profile| profile.member_since),
        }
    }
}
