//! Timeline items, snapshots, reading overlays, config, and listener traits across FFI.
//! Live/focused window handles and client opening methods live in `timeline.rs`.

use std::sync::Arc;

use papo_sdk::{HistoryGap, VisibilityContinuity};
use papo_sdk_ui::{TimelineDirection, TimelineItem, TimelinePage, TimelineReadingState};

use crate::{
    define_ffi_vector_diff,
    ids::FfiHistoryCursor,
    ids::FfiHistoryOrder,
    message_content::FfiMessageContent,
    messages::FfiMessage,
    outgoing_payload::FfiMessageTarget,
    outgoing_state::{FfiOperationFailure, FfiOperationState},
    timeline_errors::FfiHistoryClosedReason,
};

/// Foreign civil UTC offset lookup by Unix timestamp in microseconds (never wall clock).
#[uniffi::export(with_foreign)]
pub trait FfiUtcOffsetProvider: Send + Sync {
    /// Returns whole seconds east of UTC at `created_at_us`. Must be fast and nonblocking.
    fn offset_at_seconds_east(&self, created_at_us: i64) -> i32;
}

/// Configuration for opening a timeline window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineConfig {
    /// Page size from 1 through 100.
    pub page_size: u8,
    /// Bounded subscriber diff buffer capacity before emitting coherent `Reset`.
    pub subscriber_capacity: u32,
}

/// Pagination and loading indicator edge direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTimelineDirection {
    /// Extend toward older history.
    Older,
    /// Extend toward newer history.
    Newer,
}

impl From<FfiTimelineDirection> for TimelineDirection {
    fn from(v: FfiTimelineDirection) -> Self {
        match v {
            FfiTimelineDirection::Older => Self::Older,
            FfiTimelineDirection::Newer => Self::Newer,
        }
    }
}

impl From<TimelineDirection> for FfiTimelineDirection {
    fn from(v: TimelineDirection) -> Self {
        match v {
            TimelineDirection::Older => Self::Older,
            TimelineDirection::Newer => Self::Newer,
        }
    }
}

/// Visible frame continuity hint distinguishing smooth scroll from discontinuous navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiVisibilityContinuity {
    /// Scrolled continuously from previous frame in same window.
    Continuous,
    /// Initial frame or discontinuous jump (`live`, `live_at`, `focused`, hourglass, scroll jump).
    Discontinuous,
}

impl From<FfiVisibilityContinuity> for VisibilityContinuity {
    fn from(v: FfiVisibilityContinuity) -> Self {
        match v {
            FfiVisibilityContinuity::Continuous => Self::Continuous,
            FfiVisibilityContinuity::Discontinuous => Self::Discontinuous,
        }
    }
}

/// Unknown coverage boundary item displayed between timeline segments.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiHistoryGap {
    /// Exact backend boundary cursor.
    pub cursor: FfiHistoryCursor,
    /// Direction that fills this gap.
    pub order: FfiHistoryOrder,
}

impl From<&HistoryGap> for FfiHistoryGap {
    fn from(g: &HistoryGap) -> Self {
        Self {
            cursor: (&g.cursor).into(),
            order: g.order.into(),
        }
    }
}

impl From<FfiHistoryGap> for HistoryGap {
    fn from(g: FfiHistoryGap) -> Self {
        Self {
            cursor: g.cursor.into(),
            order: g.order.into(),
        }
    }
}

/// Complete timeline item variant projected directly from `papo_sdk_ui::TimelineItem`.
// UniFFI enum variants are unboxed value projections across FFI.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTimelineItem {
    /// Durable outgoing local create intent or confirmed echo awaiting canonical projection.
    LocalEcho {
        /// Stable local operation UUID string.
        id: String,
        /// Operation payload, state, attachments, and failure cause.
        operation: FfiOperationStub,
    },
    /// Authoritative history message paired with its cached parsed AST.
    Message {
        /// Canonical message UUID string.
        id: String,
        /// Authoritative wire message fields.
        message: FfiMessage,
        /// Pre-parsed CommonMark/mention AST.
        content: FfiMessageContent,
    },
    /// Civil date separator calculated using `FfiUtcOffsetProvider` at `before` timestamp.
    DateSeparator {
        /// Civil days since 1970-01-01.
        day: i64,
        /// First message UUID string following this separator (stable key across gap splits).
        before_message_id: String,
    },
    /// Unread banner separator immediately above `before_message_id`.
    UnreadMarker {
        /// First unread message UUID string.
        before_message_id: String,
    },
    /// Unknown gap placeholder carrying exact cursor needed to fill it.
    Gap {
        /// Gap boundary descriptor.
        gap: FfiHistoryGap,
    },
    /// Active pagination spinner at window edge.
    Loading {
        /// Edge direction currently fetching.
        direction: FfiTimelineDirection,
    },
}

/// Lightweight local echo projection without `Debug` restrictions breaking `FfiTimelineItem` equality.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiOperationStub {
    /// Operation fields.
    pub id: String,
    /// Channel UUID.
    pub channel_id: String,
    /// Current state.
    pub state: FfiOperationState,
    /// Attempt count.
    pub attempts: u32,
    /// Predecessor local dependency UUID if any.
    pub depends_on: Option<String>,
    /// Failure cause when `Failed`.
    pub failure: Option<FfiOperationFailure>,
    /// Confirmed backend message UUID if assigned.
    pub message_id: Option<String>,
    /// True once projected into canonical history.
    pub history_projected: bool,
    /// Authoritative content body for creates when available.
    pub preview_text: Option<String>,
    /// Reply target UUID if local echo is a reply.
    pub reply_to: Option<FfiMessageTarget>,
    /// Creation Unix microseconds.
    pub created_at_us: i64,
}

impl From<&Arc<TimelineItem>> for FfiTimelineItem {
    fn from(item: &Arc<TimelineItem>) -> Self {
        match item.as_ref() {
            TimelineItem::LocalEcho { id, operation } => {
                let (preview_text, reply_to) = match operation.kind.as_ref() {
                    papo_sdk::OperationKind::Message(msg) => (
                        Some(msg.content.clone()),
                        msg.reply_to.as_ref().map(Into::into),
                    ),
                    papo_sdk::OperationKind::Edit {
                        content, target, ..
                    } => (Some(content.clone()), Some(target.into())),
                    papo_sdk::OperationKind::Delete { target }
                    | papo_sdk::OperationKind::Reaction { target, .. } => {
                        (None, Some(target.into()))
                    }
                };
                Self::LocalEcho {
                    id: id.to_string(),
                    operation: FfiOperationStub {
                        id: operation.id.to_string(),
                        channel_id: operation.channel_id.to_string(),
                        state: operation.state.into(),
                        attempts: operation.attempts,
                        depends_on: operation.depends_on.map(|u| u.to_string()),
                        failure: operation.failure.map(Into::into),
                        message_id: operation.message_id.map(|u| u.to_string()),
                        history_projected: operation.history_projected,
                        preview_text,
                        reply_to,
                        created_at_us: operation.created_at_us,
                    },
                }
            }
            TimelineItem::Message {
                id,
                message,
                content,
            } => Self::Message {
                id: id.to_string(),
                message: message.as_ref().into(),
                content: content.as_ref().into(),
            },
            TimelineItem::DateSeparator { day, before } => Self::DateSeparator {
                day: *day,
                before_message_id: before.to_string(),
            },
            TimelineItem::UnreadMarker { before } => Self::UnreadMarker {
                before_message_id: before.to_string(),
            },
            TimelineItem::Gap(gap) => Self::Gap { gap: gap.into() },
            TimelineItem::Loading(dir) => Self::Loading {
                direction: (*dir).into(),
            },
            _ => Self::Loading {
                direction: FfiTimelineDirection::Older,
            },
        }
    }
}

define_ffi_vector_diff!(
    "Ordered diff over timeline window items.",
    FfiTimelineDiff,
    FfiTimelineItem
);

/// Atomic initial timeline window baseline captured under window lock.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineSnapshot {
    /// Window snapshot version stamped on this baseline.
    pub version: u64,
    /// Session connection epoch owning this baseline.
    pub generation: u64,
    /// Ordered timeline items (oldest to newest).
    pub items: Vec<FfiTimelineItem>,
}

/// Ordered diff batch advancing a timeline window to `version`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineUpdate {
    /// Resulting window snapshot version.
    pub version: u64,
    /// Session connection epoch.
    pub generation: u64,
    /// Ordered list edits, or `Reset` on overflow/reconnect.
    pub diffs: Vec<FfiTimelineDiff>,
}

/// Metadata returned when a pagination or gap-fill request finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelinePage {
    /// Connection epoch that accepted the page.
    pub generation: u64,
    /// Count of messages returned.
    pub message_count: u64,
    /// True when more history may exist in the paged direction.
    pub has_more: bool,
}

impl From<TimelinePage> for FfiTimelinePage {
    fn from(v: TimelinePage) -> Self {
        Self {
            generation: v.generation.get(),
            message_count: u64::try_from(v.message_count).unwrap_or(u64::MAX),
            has_more: v.has_more,
        }
    }
}

/// Listener for timeline items, ordered diffs, and typed terminal closure.
#[uniffi::export(with_foreign)]
pub trait FfiTimelineListener: Send + Sync {
    /// Initial atomic window baseline.
    fn on_snapshot(&self, snapshot: FfiTimelineSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiTimelineUpdate);
    /// Terminal closure reason (`None` when closed externally by client shutdown/dispose).
    fn on_closed(&self, reason: FfiHistoryClosedReason);
}

/// Active catch-up interval boundaries and hourglass readiness state.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineCatchUp {
    /// Exclusive start anchor message UUID string if known.
    pub start_after_id: Option<String>,
    /// First unread foreign message UUID inside interval when resolved locally.
    pub start_target_id: Option<String>,
    /// Exclusive upper bound message UUID string.
    pub end_before_id: String,
    /// True if unloaded prefix requires ascending resolution via `seek_catch_up`.
    pub needs_history: bool,
    /// True while hourglass prefix resolution is active.
    pub seeking: bool,
}

/// Behind-head live arrivals state and two-stop down-arrow navigation cursors.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineArrivals {
    /// Count of unseen foreign messages newer than window frontier.
    pub count: u64,
    /// First down-arrow stop: oldest unseen foreign message cursor.
    pub oldest_unseen: Option<FfiHistoryCursor>,
    /// Second down-arrow stop: newest live tail cursor.
    pub live_head: Option<FfiHistoryCursor>,
}

/// Combined catch-up and arrival reading overlay snapshot for a window.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTimelineReadingState {
    /// Overlay publication version.
    pub version: u64,
    /// History connection generation.
    pub generation: u64,
    /// Active catch-up interval if open.
    pub catch_up: Option<FfiTimelineCatchUp>,
    /// Behind-head arrival counters and cursors.
    pub arrivals: FfiTimelineArrivals,
}

impl From<&TimelineReadingState> for FfiTimelineReadingState {
    fn from(s: &TimelineReadingState) -> Self {
        Self {
            version: s.version,
            generation: s.generation.get(),
            catch_up: s.catch_up.as_ref().map(|c| FfiTimelineCatchUp {
                start_after_id: c.start_after_id.map(|u| u.to_string()),
                start_target_id: c.start_target_id.map(|u| u.to_string()),
                end_before_id: c.end_before_id.to_string(),
                needs_history: c.needs_history,
                seeking: c.seeking,
            }),
            arrivals: FfiTimelineArrivals {
                count: s.arrivals.count,
                oldest_unseen: s.arrivals.oldest_unseen.as_ref().map(Into::into),
                live_head: s.arrivals.live_head.as_ref().map(Into::into),
            },
        }
    }
}

/// Listener for per-window catch-up boundaries and live arrivals updates.
#[uniffi::export(with_foreign)]
pub trait FfiTimelineReadingListener: Send + Sync {
    /// Initial atomic overlay baseline.
    fn on_snapshot(&self, state: FfiTimelineReadingState);
    /// Incremental overlay updates.
    fn on_update(&self, state: FfiTimelineReadingState);
    /// Terminal closure notification.
    fn on_closed(&self, reason: FfiHistoryClosedReason);
}
