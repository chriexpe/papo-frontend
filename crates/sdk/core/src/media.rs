//! Account-authorized encoded media references; rendering, decoding and durable uploads live elsewhere.

mod cached;
mod collection;
mod download;
mod emojis;
mod eviction;
mod files;
mod index;
mod pins;
#[cfg(test)]
mod race_tests;
mod ranges;
mod service;
#[cfg(test)]
mod tests;

use crate::{
    ClientError, DataRoot, StorageError,
    storage_worker::{DatabaseWriter, WorkerError},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) use download::anonymous_client;
pub use emojis::{EmojiMetadata, EmojiPage};
pub use ranges::MediaRange;
pub use service::MediaService;

/// A remote representation. A local digest is deliberately not a retrieval capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MediaSource {
    /// Opaque backend media identifier, distinct from the local SHA-256 digest.
    Blob(String),
    /// Inline server icon returned by authenticated server metadata.
    ServerIcon,
    /// Original attachment, authorized by its message's channel.
    Attachment(crate::Uuid),
    /// Backend-generated attachment thumbnail.
    Thumbnail(crate::Uuid),
    /// Backend-relayed embed video.
    EmbedVideo(crate::Uuid),
    /// A bounded encoded range of a blob, original attachment or relayed video; partial refs never grant full access.
    Range {
        /// Original representation; nested ranges and mutable avatars are refused.
        source: Box<MediaSource>,
        /// First requested byte, inclusive.
        start_bytes: u64,
        /// Last requested byte, inclusive.
        end_bytes: u64,
    },
    /// Anonymous public avatar, keyed by an explicit revision supplied by the resource owner.
    UserAvatar {
        /// Backend user UUID.
        user: crate::Uuid,
        /// Resource revision; a changed value forces an unconditional fetch.
        revision: String,
    },
    /// Anonymous public webhook avatar, with an explicit revision.
    WebhookAvatar {
        /// Backend webhook UUID.
        webhook: crate::Uuid,
        /// Resource revision, independent of last-modified timestamp precision.
        revision: String,
    },
    /// Anonymous HTTPS media. Never uses the Papo transport or its cookie jar.
    External(String),
    /// Inline emoji image returned by an authenticated backend page.
    Emoji {
        /// Immutable backend emoji UUID.
        id: crate::Uuid,
        /// Exact backend creation timestamp.
        revision: String,
    },
}

/// Encoded bytes and representation metadata; no filesystem path or digest grants are exported.
#[derive(Clone)]
pub struct MediaData {
    /// Encoded bytes owned independently of future eviction.
    pub bytes: Arc<[u8]>,
    /// Backend/provider content type, if present.
    pub content_type: Option<String>,
    /// Validated partial representation boundaries; None denotes a complete object.
    pub range: Option<MediaRange>,
}

/// Sanitized media errors; response text, paths, URLs and credentials are excluded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MediaError {
    /// No reference for this account/representation exists locally.
    #[error("media reference unavailable")]
    Unavailable,
    /// Invalid representation, response or stored index row.
    #[error("invalid media data")]
    InvalidData,
    /// Encoded response exceeds the configured per-object limit.
    #[error("media object too large")]
    TooLarge,
    /// Blocking filesystem operation failed.
    #[error("media filesystem operation failed")]
    Filesystem,
    /// Media task stopped unexpectedly.
    #[error("media task stopped")]
    TaskStopped,
    /// Backend media transport failure.
    #[error(transparent)]
    Transport(#[from] papo_api::media::MediaError),
    /// Shared rebuildable storage failure.
    #[error(transparent)]
    Storage(#[from] WorkerError),
    /// Session lifecycle or authentication failure.
    #[error(transparent)]
    Session(#[from] ClientError),
}

impl From<rusqlite::Error> for MediaError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}
impl From<StorageError> for MediaError {
    fn from(_: StorageError) -> Self {
        Self::Filesystem
    }
}

/// Store-wide file/index exclusion. Network requests never hold this gate.
#[derive(Clone)]
pub(crate) struct MediaStore {
    pub(crate) root: DataRoot,
    pub(crate) writer: DatabaseWriter,
    gate: Arc<Mutex<()>>,
    lease: Arc<crate::data_root::ProfileLease>,
    fetches: Arc<tokio::sync::Semaphore>,
    reads: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    read_probe: Arc<std::sync::Mutex<Option<files::ReadProbe>>>,
    pins: pins::ReadPins,
    collection: Arc<std::sync::Mutex<collection::Scan>>,
}

impl MediaStore {
    pub(crate) fn new(storage: &crate::Storage) -> Result<Self, StorageError> {
        let writer =
            storage.register_rebuildable(storage.root().media_index(), index::initialize)?;
        Ok(Self {
            root: storage.root().clone(),
            writer,
            gate: Arc::default(),
            lease: Arc::clone(&storage.registry().lease),
            fetches: Arc::new(tokio::sync::Semaphore::new(2)),
            reads: Arc::new(tokio::sync::Semaphore::new(2)),
            #[cfg(test)]
            read_probe: Arc::default(),
            pins: pins::ReadPins::default(),
            collection: Arc::default(),
        })
    }
}
