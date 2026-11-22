//! Unified account-scoped direct-conversation metadata; mutations, notification policy and navigation live elsewhere.

mod projection;
#[cfg(test)]
mod tests;

use eyeball_im::VectorDiff;
use futures_util::Stream;
use imbl::Vector;
use papo_sdk::{AccountId, ClientHandle, Generation, Session, Uuid};
use std::{pin::Pin, sync::Arc};

/// Stable conversation identity includes account, even when servers reuse the same remote UUID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirectMessageId {
    /// Originating signed-in account; clients use it for navigation and requests.
    pub account: AccountId,
    /// Remote channel identity, never interpreted as a chronology key.
    pub channel: Uuid,
}

/// Direct-conversation row, without fetching media, profiles or message history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectMessageItem {
    /// Account-scoped identity.
    pub id: DirectMessageId,
    /// Resource connection generation, validated again before acting.
    pub generation: Generation,
    /// Backend participant identity.
    pub user: Uuid,
    /// Backend username.
    pub username: Option<String>,
    /// Backend nickname.
    pub nickname: Option<String>,
    /// Exact backend last-message timestamp, falling back to conversation creation.
    pub activity_at: String,
    /// SDK read-point/coverage projection. Absent until this generation has verified read state.
    pub unread: Option<papo_sdk::ReadState>,
    /// Core-owned eligible mention count and unread evidence; absent before a matching image.
    pub badges: Option<Arc<papo_sdk::ChannelBadges>>,
}

/// Atomic global baseline followed by sequential list changes.
pub struct DirectMessageSubscription {
    /// Projection version represented by the baseline.
    pub version: u64,
    /// Rows sorted by activity instant descending, then account/channel for deterministic ties.
    pub snapshot: Vector<Arc<DirectMessageItem>>,
    /// Capacity-one sources coalesce; edits reconcile directly against the last observed images.
    pub stream: Pin<Box<dyn Stream<Item = DirectMessageUpdate> + Send>>,
}

/// One unified list publication. Account revocation removes only that account's rows.
pub struct DirectMessageUpdate {
    /// Projection version, incremented once for each visible edit batch or generation reset.
    pub version: u64,
    /// Sequential edits, including Reset on a source discontinuity.
    pub diffs: Vec<VectorDiff<Arc<DirectMessageItem>>>,
}

#[derive(Clone)]
enum SourceMode {
    Sessions(Vec<Session>),
    Client(ClientHandle),
}

/// Pull-driven aggregation of direct conversations across explicit sessions or a whole `ClientHandle`.
#[derive(Clone)]
pub struct DirectMessageService {
    mode: SourceMode,
}

impl DirectMessageService {
    /// Shares explicitly supplied sessions and their watches without adding new connections.
    #[must_use]
    pub fn new(sessions: Vec<Session>) -> Self {
        Self {
            mode: SourceMode::Sessions(sessions),
        }
    }

    /// Dynamically follows all signed-in accounts on `handle` (additions, removals, reconnects).
    #[must_use]
    pub fn for_client(handle: ClientHandle) -> Self {
        Self {
            mode: SourceMode::Client(handle),
        }
    }

    /// Captures each independent account atomically with its stream, then merges activity order.
    /// Cached rows convey no permission grant; a later action must use its originating session.
    #[must_use]
    pub fn subscribe(&self) -> DirectMessageSubscription {
        match &self.mode {
            SourceMode::Sessions(sessions) => projection::subscribe_static(sessions),
            SourceMode::Client(handle) => projection::subscribe_client(handle.clone()),
        }
    }
}
