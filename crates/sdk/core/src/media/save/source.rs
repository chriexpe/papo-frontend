//! What a save writes out (an attachment's original, or an embed's relayed picture) and where
//! its bytes come from: the account's cache when it holds them, else the backend. Writing them
//! is `save`'s.

use std::sync::Arc;

use super::MAX_SAVE_BYTES;
use super::{SaveFailure, failure_of};
use crate::media::{MediaError, MediaSource};
use crate::{Session, Uuid, generation::Epoch};

/// What a save writes out.
pub(super) enum Saving {
    /// An attachment's original.
    Attachment(Uuid),
    /// An embed's relayed picture at a revision.
    EmbedImage { embed: Uuid, revision: String },
}

impl Saving {
    /// The UUID its partial file is named after.
    pub(super) fn id(&self) -> Uuid {
        match self {
            Saving::Attachment(id) | Saving::EmbedImage { embed: id, .. } => *id,
        }
    }

    /// The cache's source for it.
    pub(super) fn source(&self) -> MediaSource {
        match self {
            Saving::Attachment(id) => MediaSource::Attachment(*id),
            Saving::EmbedImage { embed, revision } => MediaSource::EmbedImage {
                embed: *embed,
                revision: revision.clone(),
            },
        }
    }
}

/// Where the original's bytes come from.
pub(super) enum Source {
    /// This account's cached copy, read whole.
    Cached(Arc<[u8]>),
    /// The backend's answer, streamed.
    Streamed(papo_api::media::MediaBody),
}

/// The original's source: the account's cache when it holds it, else the backend's answer.
pub(super) async fn open_source(
    session: &Session,
    epoch: &Epoch,
    saving: &Saving,
) -> Result<Source, SaveFailure> {
    let source = saving.source();
    let store = &session.client.inner.media;
    let cached = store
        .cached_reference(session, epoch, &source, true)
        .await
        .map_err(|error| failure_of(&error))?;
    if let Some((_, data)) = cached {
        return Ok(Source::Cached(data.bytes));
    }
    let opened = match saving {
        Saving::Attachment(attachment) => {
            let rest = crate::media::download::authenticated(session, epoch)
                .await
                .map_err(|error| failure_of(&error))?;
            tokio::time::timeout(
                session.client.inner.config.request_timeout,
                rest.attachment_body(*attachment, MAX_SAVE_BYTES),
            )
            .await
            .map_err(|_| SaveFailure::Unreachable)?
            .map(Source::Streamed)
            .map_err(MediaError::Transport)
        }
        // A relayed picture is a few megabytes in one answer, under its own limit.
        Saving::EmbedImage { .. } => {
            crate::media::download::download(session, epoch, &source, None)
                .await
                .map(|response| Source::Cached(response.bytes.into()))
        }
    };
    match opened {
        Ok(opened) => Ok(opened),
        Err(error) => {
            if crate::media::download::denied(&error) {
                let _ = store
                    .revoke_denied(session, epoch.generation, &source)
                    .await;
            }
            Err(failure_of(&error))
        }
    }
}
