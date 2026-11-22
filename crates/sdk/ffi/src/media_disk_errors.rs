//! Typed media transfer/export errors and disk budget measurement failures across FFI.
//! Core client lifecycle and authentication errors live in `errors.rs`.

use papo_sdk::{DiskError, MediaError};

use crate::{errors::FfiClientError, timeline_errors::FfiHistoryError};

/// Typed media download/cache/export error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiMediaError {
    /// Object unavailable, 404, 403, or offline miss.
    #[error("media unavailable")]
    Unavailable,
    /// Corrupt disk integrity or invalid URL/range.
    #[error("invalid media data")]
    InvalidData,
    /// Media exceeds 100 MiB transport limit.
    #[error("media too large")]
    TooLarge,
    /// Inline byte read exceeded the 4 MiB FFI byte buffer ceiling; use streaming file export.
    #[error("media exceeds inline FFI buffer limit; use fetch_to_file")]
    ExceedsInlineLimit,
    /// Supplied destination path is invalid (directory, missing parent, or filename conflict).
    #[error("invalid media destination path")]
    InvalidPath,
    /// Filesystem write, sync, or atomic publish failed.
    #[error("media filesystem error")]
    Filesystem,
    /// Media index storage failure.
    #[error("media storage error")]
    Storage,
    /// Background media task stopped.
    #[error("media task stopped")]
    TaskStopped,
    /// Session closed or generation superseded.
    #[error("media session error")]
    Session {
        /// Underlying client failure.
        error: FfiClientError,
    },
}

impl From<MediaError> for FfiMediaError {
    fn from(value: MediaError) -> Self {
        match value {
            MediaError::InvalidData => Self::InvalidData,
            MediaError::TooLarge => Self::TooLarge,
            MediaError::Filesystem => Self::Filesystem,
            MediaError::Storage(_) => Self::Storage,
            MediaError::TaskStopped => Self::TaskStopped,
            MediaError::Session(err) => Self::Session { error: err.into() },
            // Unavailable and future variants surface as Unavailable.
            _ => Self::Unavailable,
        }
    }
}

/// Typed physical disk budget / usage measurement failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiDiskError {
    /// Filesystem allocation could not be measured.
    #[error("disk usage measurement failed")]
    Measurement,
    /// Media reclamation failed.
    #[error("disk media failure")]
    Media {
        /// Underlying media error.
        error: FfiMediaError,
    },
    /// History reclamation failed.
    #[error("disk history failure")]
    History {
        /// Underlying history error.
        error: FfiHistoryError,
    },
    /// Client lifecycle failure.
    #[error("disk session failure")]
    Session {
        /// Underlying client error.
        error: FfiClientError,
    },
}

impl From<DiskError> for FfiDiskError {
    fn from(value: DiskError) -> Self {
        match value {
            DiskError::Media(err) => Self::Media { error: err.into() },
            DiskError::History(err) => Self::History { error: err.into() },
            DiskError::Session(err) => Self::Session { error: err.into() },
            // Measurement and future errors surface as Measurement.
            _ => Self::Measurement,
        }
    }
}
