//! Durable account expression metadata and optional catalog access; outgoing owns dispatch and confirmed transitions.

mod catalog;
mod persistence;
mod subscription;
pub use subscription::{ExpressionFault, ExpressionUpdate};
#[cfg(test)]
mod tests;

use crate::{ClientError, Session, StorageError};
use serde::{Deserialize, Serialize};

pub use catalog::{CatalogPage, GiphyConfig};

/// Stable saved GIF metadata. Encoded bytes use the shared media store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gif {
    /// Provider's stable opaque ID.
    pub id: String,
    /// Original HTTPS GIF URL.
    pub url: String,
    /// Provider attribution page.
    pub page_url: String,
    /// Provider title metadata, never used as an identifier.
    pub title: String,
}

impl Gif {
    pub(crate) fn validate(&self) -> Result<(), ExpressionError> {
        if self.id.is_empty() || self.id.len() > 256 || self.title.len() > 4096 {
            return Err(ExpressionError::InvalidData);
        }
        for raw in [&self.url, &self.page_url] {
            let url = url::Url::parse(raw).map_err(|_| ExpressionError::InvalidData)?;
            if raw.len() > 8192
                || url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(ExpressionError::InvalidData);
            }
        }
        Ok(())
    }
    /// Stages an ordinary durable-queue payload. Selection never records a recent.
    /// # Errors
    /// Returns invalid metadata; the server remains responsible for message acceptance.
    pub fn message(&self) -> Result<crate::OutgoingMessage, ExpressionError> {
        self.validate()?;
        Ok(crate::OutgoingMessage {
            content: self.url.clone(),
            gif: Some(self.clone()),
            ..crate::OutgoingMessage::default()
        })
    }
}

/// A stable server sticker identity/name, without retained image bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    /// Server emoji UUID.
    pub id: crate::Uuid,
    /// Last confirmed or explicitly saved name.
    pub name: String,
}

/// Durable expression metadata, isolated by account and bounded independently of media bytes.
#[derive(Clone, Default)]
pub struct ExpressionSnapshot {
    /// At most 128 saved GIFs, newest favorite first.
    pub favorite_gifs: Vec<Gif>,
    /// At most 64 confirmed sent GIFs, most recently confirmed first.
    pub recent_gifs: Vec<Gif>,
    /// At most 128 saved server sticker identities.
    pub favorite_stickers: Vec<Sticker>,
    /// At most 256 confirmed reaction usage entries, descending frequency.
    pub reaction_usage: Vec<(String, u64)>,
}

/// Catalog and durable-library failures without provider text, URLs or credentials.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExpressionError {
    /// Live catalog is not configured; saved library access remains available.
    #[error("expression catalog unavailable")]
    Unavailable,
    /// Invalid metadata, query or provider response.
    #[error("invalid expression data")]
    InvalidData,
    /// Provider request was rate limited.
    #[error("expression catalog rate limited")]
    RateLimited,
    /// Durable metadata failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Account revoked or request generation superseded.
    #[error(transparent)]
    Session(#[from] ClientError),
}
impl From<crate::storage_worker::WorkerError> for ExpressionError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::Storage(error.into())
    }
}
impl From<rusqlite::Error> for ExpressionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}

/// Account-owned expression library; contains no independent send path or image cache.
#[derive(Clone)]
pub struct ExpressionService {
    session: Session,
}
impl Session {
    /// Gets durable expression metadata and optional live catalog access.
    #[must_use]
    pub fn expressions(&self) -> ExpressionService {
        ExpressionService {
            session: self.clone(),
        }
    }
}
impl ExpressionService {
    /// Loads favorites, confirmed recents and reaction usage, including offline and without a provider key.
    /// # Errors
    /// Returns revoked account or durable storage failure.
    pub async fn snapshot(&self) -> Result<ExpressionSnapshot, ExpressionError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                let epoch = session.inner.epochs.current();
                let result = persistence::snapshot(session.clone()).await;
                session.inner.epochs.validate(epoch.generation)?;
                Ok(result)
            })
            .await?
    }
    /// Saves/removes a GIF favorite after FULL commit; selection does not affect recents.
    /// # Errors
    /// Returns invalid metadata, account revocation or durable failures.
    pub async fn favorite_gif(&self, gif: Gif, favorite: bool) -> Result<(), ExpressionError> {
        gif.validate()?;
        self.favorite(
            "gif",
            gif.id.clone(),
            serde_json::to_vec(&gif).map_err(|_| ExpressionError::InvalidData)?,
            favorite,
        )
        .await
    }
    /// Saves/removes a stable server sticker favorite after FULL commit.
    /// # Errors
    /// Returns invalid name, account revocation or durable failures.
    pub async fn favorite_sticker(
        &self,
        sticker: Sticker,
        favorite: bool,
    ) -> Result<(), ExpressionError> {
        if sticker.name.is_empty() || sticker.name.len() > 256 {
            return Err(ExpressionError::InvalidData);
        }
        self.favorite(
            "sticker",
            sticker.id.to_string(),
            serde_json::to_vec(&sticker).map_err(|_| ExpressionError::InvalidData)?,
            favorite,
        )
        .await
    }
    async fn favorite(
        &self,
        kind: &'static str,
        id: String,
        payload: Vec<u8>,
        favorite: bool,
    ) -> Result<(), ExpressionError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                let result =
                    persistence::favorite(session.clone(), kind, id, payload, favorite).await;
                if result.is_ok() {
                    session
                        .inner
                        .expression_changes
                        .send_modify(|revision| *revision = revision.saturating_add(1));
                }
                Ok(result)
            })
            .await?
    }
}

pub(crate) use persistence::confirmed;
