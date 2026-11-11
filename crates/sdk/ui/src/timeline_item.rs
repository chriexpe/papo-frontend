//! Timeline item identities and client-supplied date offsets; widgets and author grouping belong to clients.

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

/// View-ready items, with no local echoes, author grouping, platform types or translated text.
/// Unchanged items retain their `Arc`; messages retain the canonical SDK payload `Arc` too.
#[derive(Clone)]
#[non_exhaustive]
pub enum TimelineItem {
    /// An authenticated chronological message, never a detached pin/search preview.
    Message {
        /// Backend UUID, stable across edits and view movement.
        id: Uuid,
        /// Canonical immutable payload, shared with every window on this channel.
        message: Arc<Message>,
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
            (Self::Message { id: a, message: x }, Self::Message { id: b, message: y }) => {
                a == b && Arc::ptr_eq(x, y)
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
    Message(Uuid),
    Date(Uuid),
    Unread,
    Gap(String, bool),
    Loading(bool),
}
