//! History, reading, timeline, and durable outgoing error enums across FFI.
//! Core client lifecycle errors live in `errors.rs` and operation/call/search errors in
//! `operation_errors.rs`.

use papo_sdk::{HistoryClosedReason, HistoryError, OutgoingError, ReadingError};
use papo_sdk_ui::TimelineError;

use crate::errors::FfiClientError;

/// Typed closure reason for history handles and timeline streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiHistoryClosedReason {
    /// Session revoked by logout, removal, replacement, or shutdown.
    AccountClosed,
    /// Authoritative channel list no longer contains channel.
    ChannelRemoved,
    /// Gateway signaled channel deletion.
    ChannelDeleted,
    /// Server rejected history request with 403.
    Forbidden,
    /// Server returned 404.
    NotFound,
    /// SDK resource permissions denied read access.
    PermissionDenied,
}

impl From<HistoryClosedReason> for FfiHistoryClosedReason {
    fn from(value: HistoryClosedReason) -> Self {
        match value {
            HistoryClosedReason::ChannelRemoved => Self::ChannelRemoved,
            HistoryClosedReason::ChannelDeleted => Self::ChannelDeleted,
            HistoryClosedReason::Forbidden => Self::Forbidden,
            HistoryClosedReason::NotFound => Self::NotFound,
            HistoryClosedReason::PermissionDenied => Self::PermissionDenied,
            // AccountClosed and future closure reasons default cleanly to AccountClosed.
            _ => Self::AccountClosed,
        }
    }
}

/// Typed history handle failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiHistoryError {
    /// History handle closed cleanly with explicit reason.
    #[error("history closed")]
    Closed {
        /// Reason handle closed.
        reason: FfiHistoryClosedReason,
    },
    /// Channel has not yet been verified on current resource snapshot.
    #[error("history unverified")]
    Unverified,
    /// Permission evidence uncertain until revalidated.
    #[error("history access unknown")]
    AccessUnknown,
    /// Backend page failed validation.
    #[error("invalid history page")]
    InvalidPage,
    /// Snapshot version no longer matches current state.
    #[error("stale history snapshot")]
    StaleSnapshot,
    /// Cache invariant violation without cache corruption.
    #[error("history layout error")]
    Layout,
    /// Account cache storage error.
    #[error("history cache error")]
    Cache,
    /// Session/client error.
    #[error("history session error")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<&HistoryError> for FfiHistoryError {
    fn from(value: &HistoryError) -> Self {
        match value {
            HistoryError::Closed(reason) => Self::Closed {
                reason: (*reason).into(),
            },
            HistoryError::Unverified => Self::Unverified,
            HistoryError::AccessUnknown => Self::AccessUnknown,
            HistoryError::StaleSnapshot => Self::StaleSnapshot,
            HistoryError::Layout => Self::Layout,
            HistoryError::Cache(_) => Self::Cache,
            HistoryError::Session(err) => Self::Session { error: err.into() },
            // InvalidPage and future errors map to InvalidPage.
            _ => Self::InvalidPage,
        }
    }
}

impl From<HistoryError> for FfiHistoryError {
    fn from(value: HistoryError) -> Self {
        Self::from(&value)
    }
}

/// Typed reading position / visibility report failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiReadingError {
    /// Malformed chronological message key.
    #[error("invalid read point")]
    InvalidPoint,
    /// Reported IDs not in supplied snapshot version.
    #[error("invalid visible report")]
    InvalidReport,
    /// History closed or invalidated.
    #[error("reading history failure")]
    History {
        /// Underlying history failure.
        error: FfiHistoryError,
    },
    /// Durable state write failed.
    #[error("reading storage failure")]
    Storage,
    /// Client/session failure.
    #[error("reading session failure")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<&ReadingError> for FfiReadingError {
    fn from(value: &ReadingError) -> Self {
        match value {
            ReadingError::InvalidPoint => Self::InvalidPoint,
            ReadingError::InvalidReport => Self::InvalidReport,
            ReadingError::History(err) => Self::History { error: err.into() },
            ReadingError::Session(err) => Self::Session { error: err.into() },
            // Storage and future errors map to Storage.
            _ => Self::Storage,
        }
    }
}

impl From<ReadingError> for FfiReadingError {
    fn from(value: ReadingError) -> Self {
        Self::from(&value)
    }
}

/// Typed timeline failure across live/focused windows and pagination.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiTimelineError {
    /// Underlying history failure.
    #[error("timeline history failure")]
    History {
        /// Underlying history failure.
        error: FfiHistoryError,
    },
    /// Durable reading policy failure.
    #[error("timeline reading failure")]
    Reading {
        /// Underlying reading failure.
        error: FfiReadingError,
    },
    /// Outgoing snapshot subscription failure.
    #[error("timeline outgoing failure")]
    Outgoing {
        /// Underlying outgoing failure.
        error: FfiOutgoingError,
    },
    /// Invalid argument or UUID.
    #[error("invalid argument")]
    InvalidArgument,
    /// Cursor malformed or missing precision.
    #[error("invalid timeline cursor")]
    InvalidCursor,
    /// Page size or subscriber capacity invalid.
    #[error("invalid timeline config")]
    InvalidConfig,
    /// Window snapshot version/generation changed before operation.
    #[error("stale timeline snapshot")]
    StaleSnapshot,
    /// Focused anchor missing from server or cache.
    #[error("focus target missing")]
    FocusMissing,
    /// Cached anchor is known deleted.
    #[error("anchor deleted")]
    AnchorDeleted,
    /// Cached anchor exceeds live reach; open focused instead.
    #[error("anchor too far for live window")]
    AnchorTooFar,
    /// Window closed cleanly with reason.
    #[error("timeline closed")]
    Closed {
        /// Closure reason.
        reason: FfiHistoryClosedReason,
    },
    /// Version counter exhausted.
    #[error("timeline version exhausted")]
    VersionExhausted,
    /// Pagination already active in this direction.
    #[error("timeline direction busy")]
    Busy,
    /// Hourglass seek canceled cleanly.
    #[error("timeline seek cancelled")]
    Cancelled,
}

impl From<TimelineError> for FfiTimelineError {
    fn from(value: TimelineError) -> Self {
        match value {
            TimelineError::History(err) => Self::History { error: err.into() },
            TimelineError::Reading(err) => Self::Reading { error: err.into() },
            TimelineError::Outgoing(err) => Self::Outgoing {
                error: err_to_ffi(err),
            },
            TimelineError::InvalidCursor => Self::InvalidCursor,
            TimelineError::InvalidConfig => Self::InvalidConfig,
            TimelineError::FocusMissing => Self::FocusMissing,
            TimelineError::AnchorDeleted => Self::AnchorDeleted,
            TimelineError::AnchorTooFar => Self::AnchorTooFar,
            TimelineError::Closed(reason) => Self::Closed {
                reason: reason.into(),
            },
            TimelineError::VersionExhausted => Self::VersionExhausted,
            TimelineError::Busy => Self::Busy,
            TimelineError::Cancelled => Self::Cancelled,
            // StaleSnapshot and future timeline errors map to StaleSnapshot.
            _ => Self::StaleSnapshot,
        }
    }
}

/// Converts `OutgoingError` without unused warning in struct literal above.
fn err_to_ffi(err: OutgoingError) -> FfiOutgoingError {
    err.into()
}

/// Typed durable outgoing queue and staging failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiOutgoingError {
    /// Invalid content, filename, size, or reaction shape.
    #[error("invalid outgoing input")]
    InvalidInput,
    /// Caller file or source stream read failed.
    #[error("outgoing attachment source error")]
    Source,
    /// Operation, account, or dependency identity missing.
    #[error("outgoing identity not found")]
    NotFound,
    /// Operation identity already exists.
    #[error("outgoing operation already exists")]
    AlreadyExists,
    /// State does not permit requested retry or discard.
    #[error("invalid outgoing operation state")]
    InvalidState,
    /// Ambiguous `Unknown` retry requires explicit `AcceptPossibleDuplicate`.
    #[error("retry requires duplicate acknowledgement")]
    DuplicateRisk,
    /// Draft revision superseded by concurrent save.
    #[error("stale draft revision")]
    StaleDraft,
    /// Outgoing admission budget exhausted.
    #[error("outgoing queue budget exceeded")]
    Budget,
    /// Corrupt stored durable outgoing state.
    #[error("invalid durable outgoing data")]
    InvalidData,
    /// Durable storage commit failed.
    #[error("outgoing storage failure")]
    Storage,
    /// Session or transport failure.
    #[error("outgoing session failure")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<&OutgoingError> for FfiOutgoingError {
    fn from(value: &OutgoingError) -> Self {
        match value {
            OutgoingError::InvalidInput => Self::InvalidInput,
            OutgoingError::Source(_) => Self::Source,
            OutgoingError::NotFound => Self::NotFound,
            OutgoingError::AlreadyExists => Self::AlreadyExists,
            OutgoingError::InvalidState => Self::InvalidState,
            OutgoingError::DuplicateRisk => Self::DuplicateRisk,
            OutgoingError::StaleDraft => Self::StaleDraft,
            OutgoingError::Budget => Self::Budget,
            OutgoingError::InvalidData => Self::InvalidData,
            OutgoingError::Session(err) => Self::Session { error: err.into() },
            // Storage and future errors map to Storage.
            _ => Self::Storage,
        }
    }
}

impl From<OutgoingError> for FfiOutgoingError {
    fn from(value: OutgoingError) -> Self {
        Self::from(&value)
    }
}
