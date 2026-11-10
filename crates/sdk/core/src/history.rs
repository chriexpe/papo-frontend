//! Account/channel history paging and proven continuity; Timeline windows, items and read state live above it.

mod channel;
mod fetch;
pub(crate) mod live;
mod live_edge;
mod merge;
mod model;
mod mutations;
mod ordering;
mod persistence;
pub(crate) mod reconnect;
#[cfg(test)]
mod tests;

use crate::{CacheError, ClientError, Generation};
use papo_api::{HistoryCursor, HistoryOrder, wire::Message};
use std::sync::Arc;

pub use channel::HistoryHandle;
pub(crate) use channel::HistoryStore;

/// Why a history handle can no longer access its channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryClosedReason {
    /// Account removal, replacement, logout or shutdown revoked the session.
    AccountClosed,
    /// A fresh channel list no longer contains the channel.
    ChannelRemoved,
    /// The gateway reported channel deletion.
    ChannelDeleted,
    /// A history request was refused with HTTP 403.
    Forbidden,
    /// A history request returned HTTP 404.
    NotFound,
}

/// Sanitized history failures. No identifiers, content or cursor text appear in diagnostics.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoryError {
    /// The account or channel has been revoked.
    #[error("history handle closed: {0:?}")]
    Closed(HistoryClosedReason),
    /// No fresh channel-list reconciliation has authorized this channel yet.
    #[error("history channel has not been verified")]
    Unverified,
    /// The requested page violates ordering, identity, precision or progress requirements.
    #[error("invalid history page")]
    InvalidPage,
    /// Account-cache storage failed.
    #[error(transparent)]
    Cache(#[from] CacheError),
    /// Session generation, transport or lifecycle failed.
    #[error(transparent)]
    Session(ClientError),
}

impl From<ClientError> for HistoryError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Closed => Self::Closed(HistoryClosedReason::AccountClosed),
            error => Self::Session(error),
        }
    }
}

impl From<HistoryError> for ClientError {
    fn from(error: HistoryError) -> Self {
        match error {
            HistoryError::Session(error) => error,
            HistoryError::Cache(error) => Self::Cache(error),
            HistoryError::Closed(_) => Self::Closed,
            HistoryError::Unverified | HistoryError::InvalidPage => Self::ResourceData,
        }
    }
}

/// The authority used for this page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySource {
    /// Previously proven local history; freshness is reported separately.
    Cache,
    /// A server page merged in its requesting generation.
    Server,
}

/// An unknown range. The exact cursor can be passed back to the history handle to fill it.
#[derive(Clone)]
pub struct HistoryGap {
    /// Exact server boundary, excluded from the requested page.
    pub cursor: HistoryCursor,
    /// Direction that fills this gap.
    pub order: HistoryOrder,
}

/// A bounded raw history page; messages follow the requested direction, without duplicates.
/// Pins outside chronological coverage never appear here.
pub struct HistoryPage {
    /// Connection generation that requested this result.
    pub generation: Generation,
    /// Authoritative message payloads, at most the requested limit.
    pub messages: Vec<Arc<Message>>,
    /// Exact terminal request boundary, retained even if concurrent deletes removed every row.
    pub next_cursor: Option<HistoryCursor>,
    /// Cache or server origin.
    pub source: HistorySource,
    /// True when local coverage has not been revalidated in this generation.
    pub stale: bool,
    /// More messages may exist in this direction, including across an unknown gap.
    pub has_more: bool,
    /// An explicit unknown boundary encountered after these messages, if any.
    pub gap: Option<HistoryGap>,
}
