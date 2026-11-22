//! Search queries, autocomplete matches, cursors, scoped hit handles, and hydration records.
//! Interactive search surfaces and detached hydration requests live in `search.rs`.

use std::sync::Arc;

use papo_sdk::{SearchCursor, SearchHit, SearchQuery, SearchTarget};
use papo_sdk_ui::{SearchMatches, SearchUpdate};

use crate::{
    ids::{FfiAccountId, FfiHistoryCursor, FfiHistoryOrder, parse_uuid},
    messages::{FfiExternalAuthor, FfiMessage},
    operation_errors::FfiSearchError,
};

/// Explicit typed backend search query produced by caller or `parse_query`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSearchQuery {
    /// Full-text search phrase.
    pub text: Option<String>,
    /// Exact author UUID string.
    pub author_id: Option<String>,
    /// Optional channel/DM UUID scope.
    pub channel_id: Option<String>,
    /// Exact mentioned user UUID string.
    pub mention_user_id: Option<String>,
    /// Require HTTP(S) links.
    pub has_link: bool,
    /// Explicit attachment presence/absence filter.
    pub contains_attachment: Option<bool>,
    /// Inclusive UTC date start (`YYYY-MM-DD`).
    pub date_start: Option<String>,
    /// Inclusive UTC date end (`YYYY-MM-DD`).
    pub date_end: Option<String>,
    /// Sort order; default descending when `None`.
    pub order: Option<FfiHistoryOrder>,
}

impl FfiSearchQuery {
    /// Converts validated UUID fields into `papo_sdk::SearchQuery`.
    ///
    /// # Errors
    /// Returns [`FfiSearchError::InvalidData`] on malformed UUID strings.
    pub(crate) fn into_sdk(self) -> Result<SearchQuery, FfiSearchError> {
        let map_id = |opt: Option<String>| {
            opt.map(|s| parse_uuid(&s).map_err(|_| FfiSearchError::InvalidData))
                .transpose()
        };
        Ok(SearchQuery {
            text: self.text,
            author: map_id(self.author_id)?,
            channel: map_id(self.channel_id)?,
            mention: map_id(self.mention_user_id)?,
            has_link: self.has_link,
            contains_attachment: self.contains_attachment,
            date_start: self.date_start,
            date_end: self.date_end,
            order: self.order.map(Into::into),
        })
    }

    /// Converts SDK `SearchQuery` into FFI record.
    pub(crate) fn from_sdk(q: &SearchQuery) -> Self {
        Self {
            text: q.text.clone(),
            author_id: q.author.map(|u| u.to_string()),
            channel_id: q.channel.map(|u| u.to_string()),
            mention_user_id: q.mention.map(|u| u.to_string()),
            has_link: q.has_link,
            contains_attachment: q.contains_attachment,
            date_start: q.date_start.clone(),
            date_end: q.date_end.clone(),
            order: q.order.map(Into::into),
        }
    }
}

/// Matching channel and user UUID lists for composer/search filter autocomplete.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSearchMatches {
    /// Matching member user UUID strings.
    pub user_ids: Vec<String>,
    /// Matching channel UUID strings.
    pub channel_ids: Vec<String>,
}

impl From<SearchMatches> for FfiSearchMatches {
    fn from(m: SearchMatches) -> Self {
        Self {
            user_ids: m.users.into_iter().map(|u| u.to_string()).collect(),
            channel_ids: m.channels.into_iter().map(|u| u.to_string()).collect(),
        }
    }
}

/// Backend pagination cursor tuple for detached search pages.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSearchCursor {
    /// Exact backend message creation timestamp.
    pub created_at: String,
    /// Exact backend message UUID string.
    pub id: String,
}

impl From<&SearchCursor> for FfiSearchCursor {
    fn from(c: &SearchCursor) -> Self {
        Self {
            created_at: c.created_at.clone(),
            id: c.id.clone(),
        }
    }
}

impl From<FfiSearchCursor> for SearchCursor {
    fn from(c: FfiSearchCursor) -> Self {
        Self {
            created_at: c.created_at,
            id: c.id,
        }
    }
}

/// Single detached search hit (retains its opaque SDK identity handle for validation/hydration).
#[derive(Clone, uniffi::Object)]
pub struct FfiSearchHit {
    /// Account-and-generation scoped SDK hit.
    pub(crate) inner: SearchHit,
}

#[uniffi::export]
impl FfiSearchHit {
    /// Backend message UUID if present.
    #[must_use]
    pub fn id(&self) -> Option<String> {
        self.inner.result.id.clone()
    }

    /// Preview message content snippet.
    #[must_use]
    pub fn content(&self) -> Option<String> {
        self.inner.result.content.clone()
    }

    /// Original channel UUID string.
    #[must_use]
    pub fn channel_id(&self) -> Option<String> {
        self.inner.result.channel_id.clone()
    }

    /// Original channel name reported in search hit.
    #[must_use]
    pub fn channel_name(&self) -> Option<String> {
        self.inner.result.channel_name.clone()
    }

    /// Original author user UUID string.
    #[must_use]
    pub fn author_id(&self) -> Option<String> {
        self.inner.result.author_id.clone()
    }

    /// Author username.
    #[must_use]
    pub fn author_username(&self) -> Option<String> {
        self.inner.result.author_username.clone()
    }

    /// Webhook UUID if posted via webhook.
    #[must_use]
    pub fn webhook_id(&self) -> Option<String> {
        self.inner.result.webhook_id.clone()
    }

    /// Bridged external author metadata if present.
    #[must_use]
    pub fn external_author(&self) -> Option<FfiExternalAuthor> {
        self.inner.result.external_author.as_ref().map(Into::into)
    }

    /// Exact creation timestamp string.
    #[must_use]
    pub fn created_at(&self) -> Option<String> {
        self.inner.result.created_at.clone()
    }

    /// Relevance score if returned by server.
    #[must_use]
    pub fn score(&self) -> Option<f64> {
        self.inner.result.score
    }
}

/// Bounded detached search page containing scoped hits and optional continuation cursor.
#[derive(Clone, uniffi::Record)]
pub struct FfiSearchPage {
    /// Account owning these hits.
    pub account: FfiAccountId,
    /// Request generation.
    pub generation: u64,
    /// Backend-ordered detached search hits (up to 100).
    pub hits: Vec<Arc<FfiSearchHit>>,
    /// Exact next cursor if `has_more`.
    pub next_cursor: Option<FfiSearchCursor>,
}

/// Validated search hit destination passed to focused timeline open APIs.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSearchTarget {
    /// Owning account.
    pub account: FfiAccountId,
    /// Validating generation.
    pub generation: u64,
    /// Original channel UUID string.
    pub channel_id: String,
    /// Original message UUID string.
    pub message_id: String,
    /// Exact focused history cursor for `open_focused_timeline`.
    pub cursor: FfiHistoryCursor,
}

impl From<SearchTarget> for FfiSearchTarget {
    fn from(t: SearchTarget) -> Self {
        Self {
            account: FfiAccountId::from_sdk(t.account),
            generation: t.generation.get(),
            channel_id: t.channel.to_string(),
            message_id: t.message.to_string(),
            cursor: FfiHistoryCursor {
                created_at: t.created_at,
                id: t.message.to_string(),
            },
        }
    }
}

/// Observable detached search surface state.
#[derive(Clone, uniffi::Enum)]
pub enum FfiSearchState {
    /// Idle before first query or after cancel/reconnect.
    Idle,
    /// Request currently in flight.
    Loading,
    /// Latest bounded search page.
    Ready {
        /// Complete accepted search page.
        page: FfiSearchPage,
    },
    /// Query failed (check returned method error).
    Fault,
    /// Session revoked.
    Closed,
}

/// Maps `SearchUpdate` into FFI enum.
pub(crate) fn map_search_update(upd: &SearchUpdate) -> FfiSearchState {
    match upd {
        SearchUpdate::Loading => FfiSearchState::Loading,
        SearchUpdate::Ready(page) => FfiSearchState::Ready {
            page: FfiSearchPage {
                account: FfiAccountId::from_sdk(page.account),
                generation: page.generation.get(),
                hits: page
                    .hits
                    .iter()
                    .cloned()
                    .map(|inner| Arc::new(FfiSearchHit { inner }))
                    .collect(),
                next_cursor: page.next.as_ref().map(Into::into),
            },
        },
        SearchUpdate::Fault => FfiSearchState::Fault,
        SearchUpdate::Closed => FfiSearchState::Closed,
        // Idle and future search states project to Idle cleanly.
        _ => FfiSearchState::Idle,
    }
}

/// Listener for independent detached search states.
#[uniffi::export(with_foreign)]
pub trait FfiSearchListener: Send + Sync {
    /// Initial state captured on subscription.
    fn on_snapshot(&self, state: FfiSearchState);
    /// Subsequent search state updates.
    fn on_update(&self, state: FfiSearchState);
    /// Terminal notification on account revocation.
    fn on_closed(&self);
}

/// One bounded turn of detached full-message hydration around a search hit.
// UniFFI enum variants are unboxed value projections over FFI.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, uniffi::Enum)]
pub enum FfiDetachedMessageUpdate {
    /// Full authoritative message found without altering canonical history.
    Ready {
        /// Full message record.
        message: FfiMessage,
    },
    /// Four history pages examined; call `next_batch` again to continue.
    Pending,
    /// Backend history exhausted without locating message.
    Unavailable,
}
