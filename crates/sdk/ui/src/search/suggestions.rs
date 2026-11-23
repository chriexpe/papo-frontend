//! What a search filter's value can be, as a client lists it under the field: people for
//! `from:` (a DM's two only) and `mentions:` (none in a DM), channels for `in:` (none in a
//! DM), the kinds for `has:` (dates are typed); names starting with what is typed first, then names containing
//! it, in roster or channel order, ignoring case. Each carries the text that binds it when
//! picked (`SearchNames::binding`). The list's look and keys are the client's; reading the
//! field is `papo_sdk::SearchDraft`'s.

use papo_sdk::{
    ChannelKind, HasKind, ResourceSnapshot, SearchKeyword, SearchNames, SearchScope, Uuid,
};

use crate::display_name;

/// At most this many suggestions.
pub const MAX_SEARCH_SUGGESTIONS: usize = 50;

/// What a suggestion stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestedValue {
    /// A person.
    Person(Uuid),
    /// A channel.
    Channel(Uuid),
    /// A `has:` kind.
    Has(HasKind),
}

/// One value a filter can take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSuggestion {
    /// What it stands for.
    pub value: SuggestedValue,
    /// A person's nickname else username, a channel's name; none for a kind (the client names it).
    pub name: Option<String>,
    /// A person's username, shown beside the name.
    pub username: Option<String>,
    /// The value to write after the keyword's colon: `@username`, `#name` (quoted when it has
    /// spaces) or the UUID for people and channels, whichever reads back as this value alone;
    /// a kind's English spelling (a client may write any of [`HasKind::spellings`]).
    pub binding: String,
}

/// The values `typed` (the value after `keyword`'s colon, `@` or `#` allowed) finds in `scope`.
#[must_use]
pub fn search_suggestions(
    snapshot: &ResourceSnapshot,
    keyword: SearchKeyword,
    typed: &str,
    scope: SearchScope,
) -> Vec<SearchSuggestion> {
    let typed = typed.trim_start_matches(['@', '#']).to_lowercase();
    let names = SearchNames {
        resources: snapshot,
    };
    let direct = matches!(scope, SearchScope::Direct { .. });
    match keyword {
        // A DM offers no mentions and has no channels.
        SearchKeyword::Mentions | SearchKeyword::In if direct => Vec::new(),
        SearchKeyword::From | SearchKeyword::Mentions => {
            let people = names.people(scope);
            let found = people
                .into_iter()
                .filter_map(|id| person(&names, keyword, id, scope));
            ranked(found, &typed)
        }
        SearchKeyword::In => {
            let channels = snapshot
                .channels
                .iter()
                .filter(|channel| channel.kind != ChannelKind::Category)
                .filter_map(|channel| self::channel(&names, channel.id, scope));
            ranked(channels, &typed)
        }
        // Dates have their own list (`search_date_suggestions`).
        SearchKeyword::Before | SearchKeyword::After | SearchKeyword::During => Vec::new(),
        SearchKeyword::Has => HasKind::ALL
            .into_iter()
            .filter(|kind| {
                kind.spellings()
                    .iter()
                    .any(|spelling| spelling.starts_with(&typed))
            })
            .map(|kind| SearchSuggestion {
                value: SuggestedValue::Has(kind),
                name: None,
                username: None,
                binding: kind.spelling().to_owned(),
            })
            .collect(),
    }
}

/// The suggestions for an ambiguous value's candidates (`FilterValue::Ambiguous`) in `scope`,
/// in their order: the "Which Lia?" list. Candidates the snapshot no longer holds are left out.
#[must_use]
pub fn search_candidates(
    snapshot: &ResourceSnapshot,
    keyword: SearchKeyword,
    candidates: &[Uuid],
    scope: SearchScope,
) -> Vec<SearchSuggestion> {
    let names = SearchNames {
        resources: snapshot,
    };
    candidates
        .iter()
        .filter_map(|id| {
            if keyword.takes_person() {
                person(&names, keyword, *id, scope)
            } else {
                channel(&names, *id, scope)
            }
        })
        .collect()
}

fn person(
    names: &SearchNames<'_>,
    keyword: SearchKeyword,
    id: Uuid,
    scope: SearchScope,
) -> Option<SearchSuggestion> {
    let user = names.resources.users.get(id)?;
    let username = user.username.clone().filter(|name| !name.is_empty());
    let name = display_name(user.nickname.as_deref(), username.as_deref())?.to_owned();
    Some(SearchSuggestion {
        value: SuggestedValue::Person(id),
        name: Some(name),
        username,
        binding: names.binding(keyword, id, scope),
    })
}

fn channel(names: &SearchNames<'_>, id: Uuid, scope: SearchScope) -> Option<SearchSuggestion> {
    let name = names.resources.channels.get(id)?.name.clone()?;
    Some(SearchSuggestion {
        value: SuggestedValue::Channel(id),
        name: Some(name),
        username: None,
        binding: names.binding(SearchKeyword::In, id, scope),
    })
}

/// Names starting with `typed` first, then names containing it, each in the given order.
fn ranked(
    suggestions: impl Iterator<Item = SearchSuggestion>,
    typed: &str,
) -> Vec<SearchSuggestion> {
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for suggestion in suggestions {
        let names: Vec<String> = [suggestion.name.as_deref(), suggestion.username.as_deref()]
            .into_iter()
            .flatten()
            .map(str::to_lowercase)
            .collect();
        if names.iter().any(|name| name.starts_with(typed)) {
            starting.push(suggestion);
            if starting.len() >= MAX_SEARCH_SUGGESTIONS {
                break;
            }
        } else if containing.len() < MAX_SEARCH_SUGGESTIONS
            && names.iter().any(|name| name.contains(typed))
        {
            containing.push(suggestion);
        }
    }
    starting
        .into_iter()
        .chain(containing)
        .take(MAX_SEARCH_SUGGESTIONS)
        .collect()
}
