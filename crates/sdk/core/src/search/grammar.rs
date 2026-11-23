//! The backend query a search asks for and its request validation; reading the field's text is
//! `draft`'s, and server execution stays authoritative.

use super::SearchError;
use super::draft::{MAX_SEARCH_BYTES, SearchDraft, SearchNames, SearchScope};
use crate::{HistoryOrder, Uuid};
use papo_api::wire::SearchRequest;

/// Explicit backend filters; absence of a channel means server-wide non-DM search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Backend full-text terms.
    pub text: Option<String>,
    /// Exact author UUID.
    pub author: Option<Uuid>,
    /// Explicit channel/DM scope.
    pub channel: Option<Uuid>,
    /// Exact mentioned user UUID.
    pub mention: Option<Uuid>,
    /// Require HTTP(S) links.
    pub has_link: bool,
    /// Presence or absence of attachments, including false.
    pub contains_attachment: Option<bool>,
    /// Inclusive UTC date YYYY-MM-DD.
    pub date_start: Option<String>,
    /// Inclusive UTC date YYYY-MM-DD.
    pub date_end: Option<String>,
    /// Days of the reader's own calendar (`before:`, `after:`, `during:`).
    /// The backend counts UTC days, so a query holding them is refused by
    /// [`crate::Session::search`] until a client converts them with its UTC
    /// offsets (`papo_sdk_ui::localize_search`), which sets the dates above
    /// and the exact window results are trimmed to.
    pub local_days: Option<LocalDays>,
    /// Tuple ordering; default newest first.
    pub order: Option<HistoryOrder>,
}

impl SearchQuery {
    /// Parses de/from, em/in, tem/has, mentions and quoted names for a server-wide search, as
    /// [`SearchDraft::read`] does. Ambiguous or unknown names fail.
    /// # Errors
    /// Returns invalid/unknown/ambiguous/repeated filters or unmatched quotes.
    pub fn parse(raw: &str, names: &SearchNames<'_>) -> Result<Self, SearchError> {
        if raw.len() > MAX_SEARCH_BYTES {
            return Err(SearchError::InvalidData);
        }
        SearchDraft::read(raw, names, SearchScope::Server)
            .query()
            .cloned()
            .map_err(|_| SearchError::InvalidData)
    }
    /// Whether the query asks for something the backend takes: a text within
    /// its limit, at least one filter (local days count), ordered dates.
    /// # Errors
    /// Returns invalid data for anything else.
    pub(super) fn check(&self) -> Result<(), SearchError> {
        if self
            .text
            .as_ref()
            .is_some_and(|s| s.len() > MAX_SEARCH_BYTES)
        {
            return Err(SearchError::InvalidData);
        }
        let text = self.text.as_deref().filter(|s| !s.trim().is_empty());
        if text.is_none()
            && self.author.is_none()
            && self.channel.is_none()
            && self.mention.is_none()
            && !self.has_link
            && self.contains_attachment.is_none()
            && self.date_start.is_none()
            && self.date_end.is_none()
            && self.local_days.is_none()
        {
            return Err(SearchError::InvalidData);
        }
        let start = self.date_start.as_deref().map(date).transpose()?;
        let end = self.date_end.as_deref().map(date).transpose()?;
        if start.zip(end).is_some_and(|(a, b)| a > b) {
            return Err(SearchError::InvalidData);
        }
        if self.local_days.is_some_and(|days| !days.is_ordered()) {
            return Err(SearchError::InvalidData);
        }
        Ok(())
    }

    pub(super) fn into_request(self) -> Result<SearchRequest, SearchError> {
        self.check()?;
        // Local days the client has not converted would search UTC days.
        if self.local_days.is_some() {
            return Err(SearchError::InvalidData);
        }
        let text = self.text.filter(|s| !s.trim().is_empty());
        Ok(SearchRequest {
            text,
            author: self.author.map(|id| id.to_string()),
            channel_id: self.channel.map(|id| id.to_string()),
            mention: self.mention.map(|id| id.to_string()),
            has: self.has_link.then(|| "link".into()),
            order: Some(
                if self.order == Some(HistoryOrder::Asc) {
                    "asc"
                } else {
                    "desc"
                }
                .into(),
            ),
            date_start: self.date_start,
            date_end: self.date_end,
            contains_attachment: self.contains_attachment,
        })
    }
}
/// Days of the reader's own calendar, both ends inclusive; an open end is
/// unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalDays {
    /// The first day searched (`after:` the day before, `during:` the day).
    pub first: Option<chrono::NaiveDate>,
    /// The last day searched (`before:` the day after, `during:` the day).
    pub last: Option<chrono::NaiveDate>,
}

impl LocalDays {
    /// Whether the first day is not after the last.
    #[must_use]
    pub fn is_ordered(self) -> bool {
        self.first
            .zip(self.last)
            .is_none_or(|(first, last)| first <= last)
    }
}

fn date(raw: &str) -> Result<chrono::NaiveDate, SearchError> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| SearchError::InvalidData)
}
