//! The search field's text read as filters and words, each with its UTF-8 byte range, every
//! filter value resolved against lightweight current metadata, and the query the text asks for
//! within a scope. Styling, suggestion lists and the "Which…?" picker belong to clients; the
//! backend stays authoritative for matching.
//!
//! A value names a person by UUID, by `@username` (exact, case-insensitive; a nickname equal to
//! it counts when no username is), or by a name: every member whose username or nickname starts
//! with it is a candidate, so `lia` is ambiguous between Lia and Lia Moreira until one is picked.
//! A picked person is written back as `@username`, which binds. Channels resolve the same way
//! by `#name` (exact) or a name prefix, an exact name winning over longer ones.

use std::ops::Range;

mod names;
mod tokens;

pub use names::SearchNames;
use tokens::{tokens, unquote};

use super::{LocalDays, SearchQuery};
use crate::Uuid;

/// The longest search field text, in UTF-8 bytes, the backend request takes.
pub const MAX_SEARCH_BYTES: usize = 8192;

/// Candidates kept for an ambiguous value.
pub const MAX_SEARCH_CANDIDATES: usize = 20;

/// Where a search looks; a typed `in:` narrows a server search to its channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchScope {
    /// Every readable channel of the server, no DMs (the backend excludes them).
    Server,
    /// One channel; a typed `in:` replaces it.
    Channel(Uuid),
    /// One direct conversation of `account_user`: `in:` and `mentions:` do not apply (a DM
    /// offers no mentions), and people are its two.
    Direct {
        /// The conversation.
        channel: Uuid,
        /// The signed-in person, one of its two.
        account_user: Uuid,
    },
}

/// A filter keyword, in any of its spellings (`from`/`de`, `in`/`em`, `has`/`tem`, `mentions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKeyword {
    /// The author.
    From,
    /// The channel.
    In,
    /// A link or an attachment.
    Has,
    /// A mentioned person.
    Mentions,
    /// Days before a local date.
    Before,
    /// Days after a local date.
    After,
    /// A local date.
    During,
}

impl SearchKeyword {
    /// The keyword a spelling (without its colon) names, ignoring case.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        match spelling.to_lowercase().as_str() {
            "from" | "de" => Some(Self::From),
            "in" | "em" => Some(Self::In),
            "has" | "tem" => Some(Self::Has),
            "mentions" => Some(Self::Mentions),
            "before" | "antes" => Some(Self::Before),
            "after" | "depois" => Some(Self::After),
            "during" | "durante" => Some(Self::During),
            _ => None,
        }
    }

    /// Whether its values are people.
    #[must_use]
    pub const fn takes_person(self) -> bool {
        matches!(self, Self::From | Self::Mentions)
    }

    /// Whether its values are dates (`YYYY-MM-DD`, in the reader's calendar).
    #[must_use]
    pub const fn takes_date(self) -> bool {
        matches!(self, Self::Before | Self::After | Self::During)
    }
}

/// What `has:` asks for; the backend filters no attachment kind (image, video).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HasKind {
    /// An HTTP(S) link in the text.
    Link,
    /// Any attachment.
    File,
}

impl HasKind {
    /// Every kind, in the order a client lists them.
    pub const ALL: [Self; 2] = [Self::Link, Self::File];

    /// The kind a spelling names, ignoring case.
    #[must_use]
    pub fn from_spelling(spelling: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| {
            kind.spellings()
                .iter()
                .any(|known| known.eq_ignore_ascii_case(spelling))
        })
    }

    /// Every spelling it takes, English first.
    #[must_use]
    pub const fn spellings(self) -> &'static [&'static str] {
        match self {
            Self::Link => &["link"],
            Self::File => &["file", "attachment", "arquivo", "anexo"],
        }
    }

    /// The spelling written back when a client picks it.
    #[must_use]
    pub const fn spelling(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::File => "file",
        }
    }
}

/// What a filter's value resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterValue {
    /// Nothing typed after the colon yet.
    Empty,
    /// One person.
    Person(Uuid),
    /// One channel.
    Channel(Uuid),
    /// A `has:` kind.
    Has(HasKind),
    /// A date of the reader's own calendar.
    Date(chrono::NaiveDate),
    /// Several people or channels match, in roster or channel order, at most
    /// [`MAX_SEARCH_CANDIDATES`]; nothing is searched until one is picked.
    Ambiguous(Vec<Uuid>),
    /// Nothing matches.
    Unknown,
    /// The keyword does not apply in this scope (`in:` or `mentions:` in a DM).
    Unavailable,
}

/// One `keyword:value` in the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchFilter {
    /// Which keyword.
    pub keyword: SearchKeyword,
    /// The keyword and its colon (`from:`), UTF-8 bytes of the field.
    pub keyword_range: Range<usize>,
    /// The value after the colon, quotes included; empty at the colon's end when none.
    pub value_range: Range<usize>,
    /// The value without quotes or escapes.
    pub value: String,
    /// What it resolved to.
    pub resolved: FilterValue,
}

/// Why the field's text asks for no search yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchProblem {
    /// Nothing to search for.
    Empty,
    /// A quote is still open.
    UnclosedQuote,
    /// Filter `index` resolved to no single value.
    Unresolved(usize),
    /// Filter `index` repeats an earlier one's keyword (one author, one channel, one mention,
    /// one of each `has:` kind).
    Repeated(usize),
    /// Longer than the backend takes ([`MAX_SEARCH_BYTES`]).
    TooLong,
    /// The date filters leave no day (`after:` a day not before `before:`'s).
    NoDays,
}

/// The field's text, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchDraft {
    /// The filters, in field order.
    pub filters: Vec<SearchFilter>,
    /// The free words' UTF-8 byte ranges, in field order.
    pub words: Vec<Range<usize>>,
    query: Result<SearchQuery, SearchProblem>,
}

impl SearchDraft {
    /// Reads `raw` within `scope`. Never fails: problems are reported by [`Self::query`].
    #[must_use]
    pub fn read(raw: &str, names: &SearchNames<'_>, scope: SearchScope) -> Self {
        if raw.len() > MAX_SEARCH_BYTES {
            return Self {
                filters: Vec::new(),
                words: Vec::new(),
                query: Err(SearchProblem::TooLong),
            };
        }
        let (tokens, closed) = tokens(raw);
        let mut filters = Vec::new();
        let mut words = Vec::new();
        let mut texts = Vec::new();
        for token in tokens {
            let keyword = token
                .colon
                .and_then(|colon| SearchKeyword::from_spelling(&raw[token.range.start..colon]));
            let (Some(keyword), Some(colon)) = (keyword, token.colon) else {
                texts.push(unquote(&raw[token.range.clone()]));
                words.push(token.range);
                continue;
            };
            let value = unquote(&raw[colon + 1..token.range.end]);
            let resolved = names.resolve(keyword, &value, scope);
            filters.push(SearchFilter {
                keyword,
                keyword_range: token.range.start..colon + 1,
                value_range: colon + 1..token.range.end,
                value,
                resolved,
            });
        }
        let query = if closed {
            compose(&filters, &texts, scope)
        } else {
            Err(SearchProblem::UnclosedQuote)
        };
        Self {
            filters,
            words,
            query,
        }
    }

    /// The query the text asks for, or why it asks for none yet.
    /// # Errors
    /// Returns the first problem in field order; [`SearchProblem::Empty`] with nothing to search.
    pub fn query(&self) -> Result<&SearchQuery, &SearchProblem> {
        self.query.as_ref()
    }

    /// The filter whose keyword or value holds byte offset `caret` (its value's end included):
    /// the one a client suggests values for.
    #[must_use]
    pub fn filter_at(&self, caret: usize) -> Option<&SearchFilter> {
        self.filters
            .iter()
            .find(|filter| filter.keyword_range.start < caret && caret <= filter.value_range.end)
    }
}

fn compose(
    filters: &[SearchFilter],
    texts: &[String],
    scope: SearchScope,
) -> Result<SearchQuery, SearchProblem> {
    let mut query = SearchQuery::default();
    let mut typed_channel = None;
    let mut days = LocalDays::default();
    let mut during = false;
    for (index, filter) in filters.iter().enumerate() {
        let slot_taken = match (&filter.resolved, filter.keyword) {
            (FilterValue::Person(id), SearchKeyword::From) => query.author.replace(*id).is_some(),
            (FilterValue::Person(id), SearchKeyword::Mentions) => {
                query.mention.replace(*id).is_some()
            }
            (FilterValue::Channel(id), _) => typed_channel.replace(*id).is_some(),
            (FilterValue::Has(HasKind::Link), _) => std::mem::replace(&mut query.has_link, true),
            (FilterValue::Has(HasKind::File), _) => {
                query.contains_attachment.replace(true).is_some()
            }
            (FilterValue::Date(day), keyword) => {
                let Some(taken) = take_day(&mut days, &mut during, keyword, *day) else {
                    return Err(SearchProblem::Unresolved(index));
                };
                taken
            }
            _ => return Err(SearchProblem::Unresolved(index)),
        };
        if slot_taken {
            return Err(SearchProblem::Repeated(index));
        }
    }
    if !days.is_ordered() {
        return Err(SearchProblem::NoDays);
    }
    query.local_days = (days != LocalDays::default()).then_some(days);
    let text = texts.join(" ");
    query.text = (!text.trim().is_empty()).then_some(text);
    // A scope alone is not a search: something must be typed.
    if filters.is_empty() && query.text.is_none() {
        return Err(SearchProblem::Empty);
    }
    query.channel = match scope {
        SearchScope::Server => typed_channel,
        SearchScope::Channel(channel) => typed_channel.or(Some(channel)),
        SearchScope::Direct { channel, .. } => Some(channel),
    };
    if query.check().is_err() {
        return Err(SearchProblem::Empty);
    }
    Ok(query)
}

/// Date filter `keyword` on `day` narrows `days`: `before:` ends the day
/// before, `after:` starts the day after, `during:` is that day alone and
/// takes no other date filter. Returns whether its end was already taken,
/// or none when the day before or after does not exist.
fn take_day(
    days: &mut LocalDays,
    during: &mut bool,
    keyword: SearchKeyword,
    day: chrono::NaiveDate,
) -> Option<bool> {
    let taken = match keyword {
        SearchKeyword::Before => *during || days.last.replace(day.pred_opt()?).is_some(),
        SearchKeyword::After => *during || days.first.replace(day.succ_opt()?).is_some(),
        _ => {
            let taken = *during || days.first.is_some() || days.last.is_some();
            *during = true;
            days.first = Some(day);
            days.last = Some(day);
            taken
        }
    };
    Some(taken)
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
