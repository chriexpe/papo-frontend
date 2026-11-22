//! Timeline item identities and client-supplied date offsets; widgets and author grouping belong to clients.

use crate::message_content::MessageContent;
use papo_sdk::{HistoryGap, HistoryOrder, Message, Uuid};
use std::sync::Arc;

/// A validated UTC offset in whole seconds east of UTC; no timezone database is consulted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UtcOffset(i32);

impl UtcOffset {
    /// Construct a civil offset strictly between -24 and +24 hours.
    #[must_use]
    pub fn from_seconds_east(seconds_east: i32) -> Option<Self> {
        (-86_399..86_400)
            .contains(&seconds_east)
            .then_some(Self(seconds_east))
    }
    /// Whole seconds east of UTC.
    #[must_use]
    pub fn seconds_east(self) -> i32 {
        self.0
    }
}

/// Supplies the client's civil offset at a server timestamp. Implementations must be fast,
/// deterministic for that timestamp, nonblocking, and must not reenter a Timeline.
pub trait UtcOffsetProvider: Send + Sync {
    /// Offset at an exact Unix timestamp measured in microseconds, not at the current time.
    fn offset_at(&self, created_at_us: i64) -> UtcOffset;
}

impl<F: Fn(i64) -> UtcOffset + Send + Sync> UtcOffsetProvider for F {
    fn offset_at(&self, created_at_us: i64) -> UtcOffset {
        self(created_at_us)
    }
}

/// The window edge for paging and loading indicators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineDirection {
    /// Extend toward smaller `(created_at_us, UUID)` keys.
    Older,
    /// Extend toward larger `(created_at_us, UUID)` keys.
    Newer,
}

impl TimelineDirection {
    pub(crate) fn order(self) -> HistoryOrder {
        match self {
            Self::Older => HistoryOrder::Desc,
            Self::Newer => HistoryOrder::Asc,
        }
    }
}

/// What a reply's original is, as this account's history knows it. Never fetched: the backend
/// has no message-by-ID route, so an original history does not hold stays `Unknown`.
#[derive(Clone)]
#[non_exhaustive]
pub enum ReplyOriginal {
    /// Held in history (in this window or outside it), with its parsed content, shared with the
    /// original's own item when this window shows it.
    Held {
        /// The original's backend UUID.
        id: Uuid,
        /// Canonical payload.
        message: Arc<Message>,
        /// Parsed content of `message`.
        content: Arc<MessageContent>,
    },
    /// One of this account's creates the server has not confirmed yet; its echo carries
    /// this local identity.
    Unsent {
        /// The create's local operation identity.
        local: Uuid,
    },
    /// History knows it was deleted.
    Deleted,
    /// Not held: never fetched, evicted, or deleted before this cache existed. Never a deletion.
    Unknown,
}

// The payload has no `Debug`; identities are what a log or an assertion needs.
impl std::fmt::Debug for ReplyOriginal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held { id, .. } => f
                .debug_struct("Held")
                .field("id", id)
                .finish_non_exhaustive(),
            Self::Unsent { local } => f.debug_struct("Unsent").field("local", local).finish(),
            Self::Deleted => f.write_str("Deleted"),
            Self::Unknown => f.write_str("Unknown"),
        }
    }
}

impl PartialEq for ReplyOriginal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Held {
                    id: a,
                    message: x,
                    content: xc,
                },
                Self::Held {
                    id: b,
                    message: y,
                    content: yc,
                },
            ) => a == b && Arc::ptr_eq(x, y) && Arc::ptr_eq(xc, yc),
            (Self::Unsent { local: a }, Self::Unsent { local: b }) => a == b,
            (Self::Deleted, Self::Deleted) | (Self::Unknown, Self::Unknown) => true,
            _ => false,
        }
    }
}

/// View-ready history and local echoes; author grouping, platform types and strings belong to clients.
/// Unchanged items retain their `Arc`; messages retain canonical SDK payload and parsed content `Arc`s.
#[derive(Clone)]
#[non_exhaustive]
pub enum TimelineItem {
    /// Durable create intent, including a confirmed Sent receipt awaiting canonical projection.
    /// Render Sent from its authoritative receipt; it is never an ambiguous retry candidate.
    /// Unknown remains an echo even if similar content/time appears in history.
    LocalEcho {
        /// Stable local operation identity.
        id: Uuid,
        /// Committed payload and typed dispatch state; callers render their own status strings.
        operation: Arc<papo_sdk::Operation>,
        /// The original this message replies to, when it replies.
        reply: Option<ReplyOriginal>,
    },
    /// An authenticated chronological message, never a detached pin/search preview.
    Message {
        /// Backend UUID, stable across edits and view movement.
        id: Uuid,
        /// Canonical immutable payload, shared with every window on this channel.
        message: Arc<Message>,
        /// Parsed message tree built once per `(content, edited_at)` revision and shared across windows.
        content: Arc<MessageContent>,
        /// The original this message replies to, when it replies.
        reply: Option<ReplyOriginal>,
        /// This account's newest edit or deletion of the message that history has not settled
        /// yet: Queued, Sending, Failed, Unknown, or Sent until it is settled. Draw an edit's
        /// text and a deletion from it; history keeps the confirmed message meanwhile.
        pending: Option<Arc<papo_sdk::Operation>>,
    },
    /// Local civil day as signed days since 1970-01-01. `before` stabilizes repeated days across gaps.
    DateSeparator {
        /// Civil day using the client-supplied offset at this message's creation time.
        day: i64,
        /// First message following this separator, used as its stable identity.
        before: Uuid,
    },
    /// First known unread message. Across unknown coverage this is a lower bound, not a claim
    /// that the preceding gap contains no unread messages.
    UnreadMarker {
        /// First cached unread message following this marker.
        before: Uuid,
    },
    /// Unknown chronological coverage, carrying the exact server cursor that can fill it.
    Gap(HistoryGap),
    /// An admitted pagination operation, removed on success or failure.
    Loading(TimelineDirection),
}

impl TimelineItem {
    pub(crate) fn identity(&self) -> ItemIdentity {
        match self {
            Self::LocalEcho { id, .. } => ItemIdentity::Local(*id),
            Self::Message { id, .. } => ItemIdentity::Message(*id),
            Self::DateSeparator { before, .. } => ItemIdentity::Date(*before),
            Self::UnreadMarker { .. } => ItemIdentity::Unread,
            Self::Gap(gap) => {
                ItemIdentity::Gap(gap.cursor.id.clone(), gap.order == HistoryOrder::Asc)
            }
            Self::Loading(direction) => {
                ItemIdentity::Loading(*direction == TimelineDirection::Newer)
            }
        }
    }
}

impl PartialEq for TimelineItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::LocalEcho {
                    id: a,
                    operation: x,
                    reply: xr,
                },
                Self::LocalEcho {
                    id: b,
                    operation: y,
                    reply: yr,
                },
            ) => a == b && Arc::ptr_eq(x, y) && xr == yr,
            (
                Self::Message {
                    id: a,
                    message: x,
                    content: xc,
                    reply: xr,
                    pending: xp,
                },
                Self::Message {
                    id: b,
                    message: y,
                    content: yc,
                    reply: yr,
                    pending: yp,
                },
            ) => {
                a == b
                    && Arc::ptr_eq(x, y)
                    && Arc::ptr_eq(xc, yc)
                    && xr == yr
                    && match (xp, yp) {
                        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                        (None, None) => true,
                        _ => false,
                    }
            }
            (
                Self::DateSeparator { day: a, before: x },
                Self::DateSeparator { day: b, before: y },
            ) => a == b && x == y,
            (Self::UnreadMarker { before: a }, Self::UnreadMarker { before: b }) => a == b,
            (Self::Gap(a), Self::Gap(b)) => a.cursor == b.cursor && a.order == b.order,
            (Self::Loading(a), Self::Loading(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ItemIdentity {
    Local(Uuid),
    Message(Uuid),
    Date(Uuid),
    Unread,
    Gap(String, bool),
    Loading(bool),
}
