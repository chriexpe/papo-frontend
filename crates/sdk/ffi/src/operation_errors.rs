//! Expression, notification, search, REST operation, reaction, and call error enums across FFI.
//! History, reading, timeline, and outgoing errors live in `timeline_errors.rs`.

use papo_sdk::{
    ExpressionError, NotificationError, SearchError,
    operations::{OperationError, RefreshFault, reactions::ReactionError},
};

use crate::{errors::FfiClientError, timeline_errors::FfiOutgoingError};

/// Typed expression catalog/favorite failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiExpressionError {
    /// Live catalog provider unavailable or unconfigured.
    #[error("expression catalog unavailable")]
    Unavailable,
    /// Invalid expression payload or response.
    #[error("invalid expression data")]
    InvalidData,
    /// Provider rate limited.
    #[error("expression rate limited")]
    RateLimited,
    /// Durable metadata storage error.
    #[error("expression storage error")]
    Storage,
    /// Session or transport failure.
    #[error("expression session error")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<ExpressionError> for FfiExpressionError {
    fn from(value: ExpressionError) -> Self {
        match value {
            ExpressionError::InvalidData => Self::InvalidData,
            ExpressionError::RateLimited => Self::RateLimited,
            ExpressionError::Storage(_) => Self::Storage,
            ExpressionError::Session(err) => Self::Session { error: err.into() },
            // Unavailable and future errors map to Unavailable.
            _ => Self::Unavailable,
        }
    }
}

/// Typed notification inbox, policy, or push failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiNotificationError {
    /// Malformed notification record or input.
    #[error("invalid notification data")]
    InvalidData,
    /// Bounded notification queue overflowed.
    #[error("notification overflow")]
    Overflow,
    /// Session or client lifecycle failure.
    #[error("notification session failure")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<NotificationError> for FfiNotificationError {
    fn from(value: NotificationError) -> Self {
        match value {
            NotificationError::Overflow => Self::Overflow,
            NotificationError::Session(err) => Self::Session { error: err.into() },
            // InvalidData and future errors map to InvalidData.
            _ => Self::InvalidData,
        }
    }
}

/// Typed detached search failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiSearchError {
    /// Invalid query filter syntax or ambiguous name.
    #[error("invalid search query or data")]
    InvalidData,
    /// Search request or hydration canceled.
    #[error("search canceled")]
    Canceled,
    /// Session or transport failure.
    #[error("search session failure")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<SearchError> for FfiSearchError {
    fn from(value: SearchError) -> Self {
        match value {
            SearchError::Canceled => Self::Canceled,
            SearchError::Session(err) => Self::Session { error: err.into() },
            // InvalidData and future errors map to InvalidData.
            _ => Self::InvalidData,
        }
    }
}

/// Warning status for independent post-mutation metadata refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRefreshFault {
    /// Concurrent newer update already won cleanly.
    ConcurrentChange,
    /// Backend read endpoint cannot reflect mutation.
    Unavailable,
    /// Session closed after server accepted mutation.
    Closed,
    /// Session generation superseded after server accepted mutation.
    Superseded,
    /// Local rebuildable cache write failed; server mutation succeeded.
    Cache,
    /// Subsequent refresh transport failed.
    Request,
}

impl From<RefreshFault> for FfiRefreshFault {
    fn from(value: RefreshFault) -> Self {
        match value {
            RefreshFault::ConcurrentChange => Self::ConcurrentChange,
            RefreshFault::Unavailable => Self::Unavailable,
            RefreshFault::Closed => Self::Closed,
            RefreshFault::Superseded => Self::Superseded,
            RefreshFault::Cache => Self::Cache,
            // Request and future faults map to Request.
            _ => Self::Request,
        }
    }
}

/// Refresh completion outcome paired with every acknowledged operation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRefreshStatus {
    /// Operation had no local retained cache to refresh.
    NotRequired,
    /// Local metadata/history accepted the refresh.
    Updated,
    /// Server mutation succeeded; refresh warned without rolling back success.
    Warning {
        /// Cause of refresh warning.
        fault: FfiRefreshFault,
    },
}

impl From<papo_sdk::RefreshStatus> for FfiRefreshStatus {
    fn from(value: papo_sdk::RefreshStatus) -> Self {
        match value {
            papo_sdk::RefreshStatus::Updated => Self::Updated,
            papo_sdk::RefreshStatus::Warning(fault) => Self::Warning {
                fault: fault.into(),
            },
            // NotRequired and future refresh statuses collapse to NotRequired.
            _ => Self::NotRequired,
        }
    }
}

/// Typed acknowledged REST operation failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiOperationError {
    /// Caller supplied invalid input.
    #[error("invalid operation input")]
    InvalidInput,
    /// Backend returned malformed response data.
    #[error("invalid operation data")]
    InvalidData,
    /// Target resource absent.
    #[error("operation target not found")]
    NotFound,
    /// Post-dispatch outcome unknown; never automatically retry.
    #[error("operation outcome unknown")]
    Unknown {
        /// Underlying client failure.
        error: FfiClientError,
    },
    /// Definite rejection or pre-dispatch lifecycle failure.
    #[error("operation session failure")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<OperationError> for FfiOperationError {
    fn from(value: OperationError) -> Self {
        match value {
            OperationError::InvalidInput => Self::InvalidInput,
            OperationError::NotFound => Self::NotFound,
            OperationError::Unknown(err) => Self::Unknown { error: err.into() },
            OperationError::Session(err) => Self::Session { error: err.into() },
            // InvalidData and future errors map to InvalidData.
            _ => Self::InvalidData,
        }
    }
}

/// Typed ephemeral reaction admission failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiReactionError {
    /// Invalid emoji or target input.
    #[error("invalid reaction input")]
    InvalidInput,
    /// Command lane full.
    #[error("reaction command queue busy")]
    Busy,
    /// Account closed.
    #[error("reaction account closed")]
    Closed,
    /// Reconnect superseded pending tap.
    #[error("reaction generation superseded")]
    Superseded,
    /// Durable outgoing admission failure.
    #[error("reaction durable failure")]
    Outgoing {
        /// Underlying outgoing failure.
        error: FfiOutgoingError,
    },
}

impl From<ReactionError> for FfiReactionError {
    fn from(value: ReactionError) -> Self {
        match value {
            ReactionError::InvalidInput => Self::InvalidInput,
            ReactionError::Busy => Self::Busy,
            ReactionError::Superseded => Self::Superseded,
            ReactionError::Outgoing(err) => Self::Outgoing {
                error: err.as_ref().into(),
            },
            // Closed and future variants collapse to Closed.
            _ => Self::Closed,
        }
    }
}

/// Typed call failure across join, switch, controls, and binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiCallError {
    /// Caller supplied invalid UUID.
    #[error("invalid call argument")]
    InvalidArgument,
    /// Another account owns the active call lease.
    #[error("another account holds call lease")]
    Busy,
    /// Session is offline.
    #[error("session is offline")]
    Offline,
    /// Voice connect permission denied.
    #[error("voice permission denied")]
    Forbidden,
    /// Voice permission uncertain or missing.
    #[error("voice permission unknown")]
    PermissionUnknown,
    /// Channel is not a voice channel.
    #[error("not a voice channel")]
    NotVoiceChannel,
    /// No media engine factory configured.
    #[error("media engine unavailable")]
    MediaUnavailable,
    /// Previous media lease drain still pending.
    #[error("previous call media release pending")]
    ReleasePending,
    /// Call attempt already ended.
    #[error("call attempt ended")]
    Ended,
    /// Target account session missing.
    #[error("account session not found")]
    AccountNotFound,
    /// Client or session closed.
    #[error("session closed")]
    Closed,
}

impl From<papo_sdk::CallError> for FfiCallError {
    fn from(value: papo_sdk::CallError) -> Self {
        match value {
            papo_sdk::CallError::Busy => Self::Busy,
            papo_sdk::CallError::Offline => Self::Offline,
            papo_sdk::CallError::Forbidden => Self::Forbidden,
            papo_sdk::CallError::PermissionUnknown => Self::PermissionUnknown,
            papo_sdk::CallError::NotVoiceChannel => Self::NotVoiceChannel,
            papo_sdk::CallError::MediaUnavailable => Self::MediaUnavailable,
            papo_sdk::CallError::ReleasePending => Self::ReleasePending,
            papo_sdk::CallError::Ended => Self::Ended,
            papo_sdk::CallError::AccountNotFound => Self::AccountNotFound,
            // Closed and future variants surface as Closed.
            _ => Self::Closed,
        }
    }
}
