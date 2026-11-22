//! The `@` being typed in a composer and the people it can mention, ranked: names starting
//! with what follows the `@` first, then names containing it, in roster order, ignoring case;
//! `@everyone` leads where the account may notify everyone. The picker, its keys and its
//! looks are the client's; encoding a picked mention is `message_content`'s.

use std::ops::Range;

use papo_sdk::{MentionTarget, ResourceSnapshot, Uuid, permissions::global_permissions};

/// At most this many suggestions.
pub const MAX_MENTION_SUGGESTIONS: usize = 50;

/// What follows an `@` longer than this is no longer taken as a name being typed.
const MAX_QUERY_CHARS: usize = 32;

/// The broadcast name, as suggested and as typed.
const EVERYONE: &str = "everyone";

/// An `@` being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionQuery {
    /// UTF-8 byte range from the `@` to the caret; a picked mention replaces it.
    pub byte_range: Range<usize>,
    /// What follows the `@`.
    pub text: String,
}

/// One person (or everyone) an `@` can mention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionSuggestion {
    /// Who the mention names.
    pub target: MentionTarget,
    /// The nickname, else the username (`everyone` for everyone): what the mention shows.
    pub name: String,
    /// The username, shown beside the name; none for everyone.
    pub username: Option<String>,
}

/// What a person is called: the nickname, else the username; an empty one counts as none.
#[must_use]
pub fn display_name<'a>(nickname: Option<&'a str>, username: Option<&'a str>) -> Option<&'a str> {
    nickname
        .filter(|name| !name.is_empty())
        .or(username.filter(|name| !name.is_empty()))
}

/// The `@` query a collapsed caret at byte offset `caret` of `text` ends, if any: an `@` that
/// starts a word, with no whitespace between it and the caret, outside every mention already
/// made (`mentions`, UTF-8 byte ranges). An `@` inside a word is an address, not a mention.
#[must_use]
pub fn mention_query(text: &str, caret: usize, mentions: &[Range<usize>]) -> Option<MentionQuery> {
    let before = text.get(..caret)?;
    let at = before.rfind('@')?;
    let typed = &before[at + 1..];
    if typed.chars().any(char::is_whitespace) || typed.chars().count() > MAX_QUERY_CHARS {
        return None;
    }
    if before[..at]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    if mentions.iter().any(|range| range.contains(&at)) {
        return None;
    }
    Some(MentionQuery {
        byte_range: at..caret,
        text: typed.to_owned(),
    })
}

/// The people of `snapshot`'s roster that `query` finds, ranked, for `account_user`.
/// `@everyone` leads when it matches and the account's global permissions allow it; unknown
/// permissions leave it out.
#[must_use]
pub fn mention_suggestions(
    snapshot: &ResourceSnapshot,
    account_user: Uuid,
    query: &str,
) -> Vec<MentionSuggestion> {
    let query = query.to_lowercase();
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for id in snapshot.members.iter() {
        let Some(user) = snapshot.users.get(id) else {
            continue;
        };
        let Some(name) = display_name(user.nickname.as_deref(), user.username.as_deref()) else {
            continue;
        };
        let lowered: Vec<String> = [Some(name), user.username.as_deref()]
            .into_iter()
            .flatten()
            .map(str::to_lowercase)
            .collect();
        let suggestion = || MentionSuggestion {
            target: MentionTarget::User(id),
            name: name.to_owned(),
            username: user.username.clone(),
        };
        if lowered.iter().any(|name| name.starts_with(&query)) {
            starting.push(suggestion());
        } else if containing.len() < MAX_MENTION_SUGGESTIONS
            && lowered.iter().any(|name| name.contains(&query))
        {
            containing.push(suggestion());
        }
        if starting.len() >= MAX_MENTION_SUGGESTIONS {
            break;
        }
    }
    let everyone_allowed =
        global_permissions(snapshot, account_user).is_ok_and(|granted| granted.everyone_message);
    let everyone = (everyone_allowed && EVERYONE.starts_with(&query)).then(|| MentionSuggestion {
        target: MentionTarget::Everyone,
        name: EVERYONE.to_owned(),
        username: None,
    });
    everyone
        .into_iter()
        .chain(starting)
        .chain(containing)
        .take(MAX_MENTION_SUGGESTIONS)
        .collect()
}

#[cfg(test)]
mod tests;
