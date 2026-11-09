//! Storage boundary failures; retry and presentation policy belong to callers.

/// A durable-state operation failed without acknowledging a commit.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Another owner holds the data root's exclusive process lock.
    #[error("profile in use")]
    ProfileInUse,
    /// Opening or shutting down storage requires a thread outside Tokio.
    #[error("storage lifecycle called inside a runtime")]
    RuntimeContext,
    /// The durable schema is newer than this SDK; it was not modified.
    #[error("unsupported durable schema version {found}")]
    NewerSchema {
        /// Version read from the durable database.
        found: i64,
    },
    /// A filesystem operation failed.
    #[error("storage filesystem operation failed")]
    Io(#[from] std::io::Error),
    /// SQLite rejected an operation.
    #[error("durable database operation failed")]
    Sqlite(#[from] rusqlite::Error),
    /// A storage worker rejected or failed an operation.
    #[error("storage worker operation failed")]
    Worker(#[from] crate::storage_worker::WorkerError),
    /// A persisted row violated an SDK invariant.
    #[error("invalid durable state")]
    InvalidState,
    /// A registry entry is absent or already hidden by removal.
    #[error("registry entry unavailable")]
    NotFound,
    /// Reordering requires each visible entry exactly once.
    #[error("invalid registry order")]
    InvalidOrder,
    /// A registry identity already exists, including an unfinished removal.
    #[error("registry entry already exists")]
    AlreadyExists,
}
