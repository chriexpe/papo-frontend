//! Typed client lifecycle failures; transport and storage retain their own error categories.

use crate::{ServerAddressError, StorageError, secret::SecretError};

/// SDK failures contain no credentials, addresses, server text or message contents.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// Another owner holds this profile; no session was started.
    #[error("profile already in use")]
    ProfileInUse,
    /// A lifecycle call requiring an external thread ran inside Tokio.
    #[error("client lifecycle requires an external thread")]
    RuntimeContext,
    /// Runtime creation or an owned task failed.
    #[error("client executor failed")]
    Executor,
    /// Work admission has ended.
    #[error("client or session closed")]
    Closed,
    /// A newer generation replaced this operation; its result was discarded.
    #[error("session generation superseded")]
    Superseded,
    /// The requested identity is unavailable.
    #[error("identity not found")]
    NotFound,
    /// Authenticated identity did not match the expected account.
    #[error("account identity mismatch")]
    Identity,
    /// An authoritative resource lacked a required identity or pagination boundary.
    #[error("invalid resource response")]
    ResourceData,
    /// Reconcile could not keep up with the bounded event journal; a new generation is required.
    #[error("reconcile event journal overflow")]
    ReconcileOverflow,
    /// Rebuildable account storage failed.
    #[error(transparent)]
    Cache(#[from] crate::CacheError),
    /// Required typed response fields or saved credentials were malformed.
    #[error("invalid authentication response")]
    InvalidAuth,
    /// The caller must sign in before this operation.
    #[error("account signed out")]
    SignedOut,
    /// The request deadline expired; authentication mutations may have reached the server.
    #[error("request timed out")]
    Timeout,
    /// Invalid worker, deadline or backoff configuration.
    #[error("invalid client configuration")]
    Configuration,
    /// Input address failed canonical validation.
    #[error(transparent)]
    Address(#[from] ServerAddressError),
    /// Durable storage failed.
    #[error(transparent)]
    Storage(StorageError),
    /// Credential access failed, possibly with an uncertain mutation outcome.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// A typed transport failure; its diagnostic formatting is sanitized.
    #[error(transparent)]
    Api(#[from] papo_api::Error),
    /// A sanitized single-socket gateway failure.
    #[error(transparent)]
    Gateway(#[from] papo_api::gateway::Error),
}

impl From<StorageError> for ClientError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::ProfileInUse => Self::ProfileInUse,
            StorageError::RuntimeContext => Self::RuntimeContext,
            error => Self::Storage(error),
        }
    }
}
