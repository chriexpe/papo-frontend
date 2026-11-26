//! The account's custom emoji catalog: the server's emoji names and IDs, kept on the device so a
//! relaunch resolves `:name:` before any request, and refreshed by paging the server's list at
//! each connection and when an emoji's image is missing from the media store.
//!
//! Images are never held here: listing installs each one into the shared media store under its
//! [`MediaSource::Emoji`] source ([`MediaService::emojis`]), and clients fetch them from there.
//! Matching message text against the catalog is `papo-sdk-ui`'s.
//!
//! The backend sends no event when an emoji is created or deleted, so a change made elsewhere
//! shows at the next connection.
//!
//! [`MediaService::emojis`]: crate::MediaService::emojis

mod persistence;
mod refresh;

use crate::{ClientError, MediaSource, Session, StorageError, Uuid};
use std::sync::Arc;
use tokio::sync::watch;

pub(crate) use refresh::{Shared, refresh_for_missing_image};

/// One custom emoji of the account's server. Its image is in the media store under
/// [`CustomEmoji::source`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmoji {
    /// Backend emoji UUID, the identity reactions and resolved content carry.
    pub id: Uuid,
    /// Shortcode name without its colons, as the server stores it.
    pub name: String,
    /// Encoded image format (`png`, `gif`, `webp`, …); clients choose the decoder.
    pub format: String,
    /// Creation time exactly as the server sent it; also the image's revision.
    pub created_at: String,
}

impl CustomEmoji {
    /// Where the emoji's image is in the media store.
    #[must_use]
    pub fn source(&self) -> MediaSource {
        MediaSource::Emoji {
            id: self.id,
            revision: self.created_at.clone(),
        }
    }
}

/// The catalog as last confirmed by the server, in the server's order (oldest first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmojiCatalog {
    /// Changes whenever the emoji listed change, within this process. Parsed content resolved
    /// against an older revision must be resolved again.
    pub revision: u64,
    /// Every emoji of the server, oldest first.
    pub emojis: Vec<CustomEmoji>,
}

impl EmojiCatalog {
    /// The emoji with backend ID `id`, if the server still has it.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&CustomEmoji> {
        self.emojis.iter().find(|emoji| emoji.id == id)
    }
}

/// The catalog, or why it can't be read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EmojiCatalogUpdate {
    /// The catalog this device holds; empty until the first listing when nothing was kept.
    Ready(Arc<EmojiCatalog>),
    /// The kept catalog could not be read; a later successful listing replaces this.
    Fault,
    /// The account was removed or replaced, or the client shut down.
    Closed,
}

/// Why the catalog could not be read or refreshed. Carries no server text.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmojiCatalogError {
    /// Listing the server's emoji failed (backend, transport, size or format).
    #[error(transparent)]
    Media(#[from] crate::MediaError),
    /// The server lists more emoji than a catalog holds ([`MAX_CUSTOM_EMOJI`]).
    #[error("too many custom emoji")]
    TooMany,
    /// The kept catalog failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Account revoked or connection superseded while listing.
    #[error(transparent)]
    Session(#[from] ClientError),
}

impl From<crate::storage_worker::WorkerError> for EmojiCatalogError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::Storage(error.into())
    }
}

impl From<rusqlite::Error> for EmojiCatalogError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}

/// The most custom emoji a catalog holds: 200 pages of the server's 25.
pub const MAX_CUSTOM_EMOJI: usize = 5000;

/// The account's custom emoji catalog.
#[derive(Clone)]
pub struct EmojiCatalogService {
    session: Session,
}

impl Session {
    /// Gets this account's custom emoji catalog.
    #[must_use]
    pub fn emoji_catalog(&self) -> EmojiCatalogService {
        EmojiCatalogService {
            session: self.clone(),
        }
    }
}

impl EmojiCatalogService {
    /// Subscribes to the catalog. The first subscription of a session reads the kept catalog,
    /// then keeps it fresh: it lists the server's emoji whenever a new connection comes online.
    /// # Errors
    /// Returns account revocation or executor closure. A kept catalog that can't be read is
    /// [`EmojiCatalogUpdate::Fault`], not an error.
    pub async fn subscribe(&self) -> Result<watch::Receiver<EmojiCatalogUpdate>, ClientError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                if session.is_closed() {
                    return Err(ClientError::Closed);
                }
                refresh::start(&session).await;
                Ok(session.inner.emoji_catalog.subscribe())
            })
            .await
    }

    /// Lists the server's emoji now and replaces the kept catalog when they changed. A refresh
    /// already running is joined rather than repeated. A failure keeps the catalog as it was.
    /// # Errors
    /// Returns listing, storage or session failures.
    pub async fn refresh(&self) -> Result<(), EmojiCatalogError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move { Ok(refresh::refresh(&session).await) })
            .await?
    }
}
