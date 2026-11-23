//! Resolving a filter value to the people or channels it names, from lightweight current
//! metadata, by the rules `draft` states (UUID, `@username`, `#channel`, a name prefix), and the
//! text that names one of them unambiguously when a client writes it back.

use super::{FilterValue, MAX_SEARCH_CANDIDATES, SearchKeyword, SearchScope, unquote};
use crate::search::HasKind;
use crate::{ChannelKind, ResourceSnapshot, Uuid};

/// Borrowed lightweight labels used for parsing/local filtering; never fetches profiles or a roster.
pub struct SearchNames<'a> {
    /// Current account's lightweight resource image.
    pub resources: &'a ResourceSnapshot,
}

impl SearchNames<'_> {
    pub(super) fn resolve(
        &self,
        keyword: SearchKeyword,
        value: &str,
        scope: SearchScope,
    ) -> FilterValue {
        // A lone `@` or `#` is a name still being typed.
        if value.is_empty() || value == "@" || value == "#" {
            return FilterValue::Empty;
        }
        let direct = matches!(scope, SearchScope::Direct { .. });
        match keyword {
            // A DM offers no mentions and has no channels.
            SearchKeyword::Mentions | SearchKeyword::In if direct => FilterValue::Unavailable,
            SearchKeyword::From | SearchKeyword::Mentions => self.person(value, scope),
            SearchKeyword::In => self.channel(value),
            SearchKeyword::Has => {
                HasKind::from_spelling(value).map_or(FilterValue::Unknown, FilterValue::Has)
            }
            SearchKeyword::Before | SearchKeyword::After | SearchKeyword::During => {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_or(FilterValue::Unknown, FilterValue::Date)
            }
        }
    }

    /// The people a value can name within `scope`, in roster order, each once: the DM's two
    /// (one, in a DM with yourself), else the roster.
    #[must_use]
    pub fn people(&self, scope: SearchScope) -> Vec<Uuid> {
        let resources = self.resources;
        match scope {
            SearchScope::Direct {
                channel,
                account_user,
            } => {
                let peer = resources
                    .dms
                    .get(channel)
                    .map(|dm| dm.peer_id)
                    .filter(|peer| *peer != account_user);
                [Some(account_user), peer].into_iter().flatten().collect()
            }
            _ => resources.members.iter().collect(),
        }
    }

    /// The value text that names person or channel `id` for `keyword` in `scope` and nothing
    /// else: `@username` or `#name` (quoted when it has spaces or quotes) when that reads back as
    /// `id` alone, else the UUID (two handles may differ only in case).
    #[must_use]
    pub fn binding(&self, keyword: SearchKeyword, id: Uuid, scope: SearchScope) -> String {
        let readable = if keyword.takes_person() {
            self.resources
                .users
                .get(id)
                .and_then(|user| user.username.as_deref())
                .filter(|name| !name.is_empty())
                .map(|name| quoted(&format!("@{name}")))
        } else {
            self.resources
                .channels
                .get(id)
                .and_then(|channel| channel.name.as_deref())
                .filter(|name| !name.is_empty())
                .map(|name| quoted(&format!("#{name}")))
        };
        let expected = if keyword.takes_person() {
            FilterValue::Person(id)
        } else {
            FilterValue::Channel(id)
        };
        match readable {
            Some(text) if self.resolve(keyword, &unquote(&text), scope) == expected => text,
            _ => id.to_string(),
        }
    }

    fn person(&self, raw: &str, scope: SearchScope) -> FilterValue {
        let people = self.people(scope);
        if let Ok(id) = raw.parse::<Uuid>() {
            // In a DM only its two people can be named; elsewhere the backend judges the UUID.
            let outside_dm = matches!(scope, SearchScope::Direct { .. }) && !people.contains(&id);
            return if outside_dm {
                FilterValue::Unknown
            } else {
                FilterValue::Person(id)
            };
        }
        let users = &self.resources.users;
        let named = || {
            people
                .iter()
                .filter_map(|id| users.get(*id).map(|user| (*id, user)))
        };
        if let Some(handle) = raw.strip_prefix('@') {
            let handle = handle.to_lowercase();
            let by_username: Vec<Uuid> = named()
                .filter(|(_, user)| same_lowered(user.username.as_deref(), &handle))
                .map(|(id, _)| id)
                .collect();
            if !by_username.is_empty() {
                return outcome(by_username, FilterValue::Person);
            }
            let by_nickname = named()
                .filter(|(_, user)| same_lowered(user.nickname.as_deref(), &handle))
                .map(|(id, _)| id)
                .collect();
            return outcome(by_nickname, FilterValue::Person);
        }
        let prefix = raw.to_lowercase();
        let starting = named()
            .filter(|(_, user)| {
                [user.username.as_deref(), user.nickname.as_deref()]
                    .into_iter()
                    .any(|name| starts_lowered(name, &prefix))
            })
            .map(|(id, _)| id)
            .collect();
        outcome(starting, FilterValue::Person)
    }

    fn channel(&self, raw: &str) -> FilterValue {
        if let Ok(id) = raw.parse::<Uuid>() {
            return FilterValue::Channel(id);
        }
        let searchable = || {
            self.resources
                .channels
                .iter()
                .filter(|channel| channel.kind != ChannelKind::Category)
        };
        let (name, exact_only) = match raw.strip_prefix('#') {
            Some(name) => (name.to_lowercase(), true),
            None => (raw.to_lowercase(), false),
        };
        let exact: Vec<Uuid> = searchable()
            .filter(|channel| same_lowered(channel.name.as_deref(), &name))
            .map(|channel| channel.id)
            .collect();
        if !exact.is_empty() || exact_only {
            return outcome(exact, FilterValue::Channel);
        }
        let starting = searchable()
            .filter(|channel| starts_lowered(channel.name.as_deref(), &name))
            .map(|channel| channel.id)
            .collect();
        outcome(starting, FilterValue::Channel)
    }

    /// Finds channel and person metadata by name without loading additional members/profiles.
    #[must_use]
    pub fn matching(&self, text: &str) -> (Vec<Uuid>, Vec<Uuid>) {
        let needle = text.to_lowercase();
        let channels = self
            .resources
            .channels
            .iter()
            .filter(|c| {
                c.name
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
            })
            .map(|c| c.id)
            .take(100)
            .collect();
        let people = self
            .resources
            .members
            .iter()
            .filter_map(|id| self.resources.users.get(id))
            .filter(|p| {
                [p.username.as_ref(), p.nickname.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|s| s.to_lowercase().contains(&needle))
            })
            .map(|p| p.id)
            .take(100)
            .collect();
        (channels, people)
    }
}

/// A name's lowercase characters, without allocating.
fn lowered(name: &str) -> impl Iterator<Item = char> + '_ {
    name.chars().flat_map(char::to_lowercase)
}

/// Whether `name`, lowercased, is `lower`; an empty name is no name.
fn same_lowered(name: Option<&str>, lower: &str) -> bool {
    name.is_some_and(|name| !name.is_empty() && lowered(name).eq(lower.chars()))
}

/// Whether `name`, lowercased, starts with `lower`; an empty name is no name.
fn starts_lowered(name: Option<&str>, lower: &str) -> bool {
    name.is_some_and(|name| {
        let mut name = lowered(name);
        !lower.is_empty() && lower.chars().all(|wanted| name.next() == Some(wanted))
    })
}

/// `text` as one field value: quoted, with `\` and `"` escaped, when it has spaces or quotes.
fn quoted(text: &str) -> String {
    if text.contains(char::is_whitespace) || text.contains(['"', '\\']) {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        text.to_owned()
    }
}

/// One, several or none, as a filter value. `ids` come from tables of unique ids, so none repeats.
fn outcome(mut ids: Vec<Uuid>, one: fn(Uuid) -> FilterValue) -> FilterValue {
    match ids.len() {
        0 => FilterValue::Unknown,
        1 => one(ids[0]),
        _ => {
            ids.truncate(MAX_SEARCH_CANDIDATES);
            FilterValue::Ambiguous(ids)
        }
    }
}
