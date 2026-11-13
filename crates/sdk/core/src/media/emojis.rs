//! One bounded authenticated emoji page, moving inline image bytes into the shared media store.

use super::{MediaError, MediaService, MediaSource, index::Reference};
use crate::{Generated, HistoryCursor};
use base64::Engine;

/// One metadata-only emoji entry; encoded image bytes are retrieved through its account-scoped source.
#[derive(Clone)]
pub struct EmojiMetadata {
    /// Stable backend emoji UUID.
    pub id: crate::Uuid,
    /// Backend emoji name.
    pub name: String,
    /// Encoded image format; clients choose decoders.
    pub format: String,
    /// Exact creation timestamp, preserved for pagination.
    pub created_at: String,
    /// Reference to the shared encoded store.
    pub source: MediaSource,
}
/// One bounded server emoji page with exact tuple pagination.
#[derive(Clone)]
pub struct EmojiPage {
    /// At most 25 metadata-only entries.
    pub emojis: Vec<EmojiMetadata>,
    /// Exact continuation boundary, absent after the last page.
    pub next_cursor: Option<HistoryCursor>,
}

impl MediaService {
    /// Fetches one authenticated page and commits each image's scoped ref; retains no base64 roster.
    /// # Errors
    /// Returns invalid pages, revoked generations, backend, file or index failures.
    pub async fn emojis(
        &self,
        cursor: Option<HistoryCursor>,
    ) -> Result<Generated<EmojiPage>, MediaError> {
        self.run(move |session, epoch| async move {
            let rest = super::download::authenticated(&session, &epoch).await?;
            let query = cursor.as_ref().map(|cursor| papo_api::Cursor {
                since: &cursor.created_at,
                last_id: Some(&cursor.id),
                order: Some(papo_api::HistoryOrder::Asc),
            });
            let page = tokio::select! {
                biased;
                ()=epoch.cancel.cancelled()=>return Err(crate::ClientError::Superseded.into()),
                result=session.client.inner.request(rest.emojis(query.as_ref()))=>result?,
            };
            let images = page.emojis.ok_or(MediaError::InvalidData)?;
            if images.len() > 25 || page.has_more.is_none() {
                return Err(MediaError::InvalidData);
            }
            let mut emojis = Vec::new();
            for mut image in images {
                let id = image
                    .id
                    .as_deref()
                    .ok_or(MediaError::InvalidData)?
                    .parse()
                    .map_err(|_| MediaError::InvalidData)?;
                let created_at = image.created_at.take().ok_or(MediaError::InvalidData)?;
                chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|_| MediaError::InvalidData)?;
                let name = image.name.take().ok_or(MediaError::InvalidData)?;
                let format = image.format.take().ok_or(MediaError::InvalidData)?;
                let source = MediaSource::Emoji {
                    id,
                    revision: created_at.clone(),
                };
                let raw = image.image_blob.take().ok_or(MediaError::InvalidData)?;
                if raw.len() > 350_000 {
                    return Err(MediaError::TooLarge);
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(raw)
                    .map_err(|_| MediaError::InvalidData)?;
                if bytes.len() > 256 * 1024 {
                    return Err(MediaError::TooLarge);
                }
                let store = &session.client.inner.media;
                let _gate = store.gate.lock().await;
                let digest = store.install(bytes.into()).await?;
                store
                    .grant(
                        session.clone(),
                        epoch.generation,
                        source.clone(),
                        Reference {
                            digest,
                            content_type: Some(format!("image/{format}")),
                            etag: None,
                            modified: None,
                            range: None,
                        },
                    )
                    .await?;
                emojis.push(EmojiMetadata {
                    id,
                    name,
                    format,
                    created_at,
                    source,
                });
            }
            let next_cursor = if page.has_more == Some(true) {
                let last = emojis.last().ok_or(MediaError::InvalidData)?;
                let next = HistoryCursor {
                    created_at: last.created_at.clone(),
                    id: last.id.to_string(),
                };
                if cursor
                    .as_ref()
                    .is_some_and(|old| old.created_at == next.created_at && old.id == next.id)
                {
                    return Err(MediaError::InvalidData);
                }
                Some(next)
            } else {
                None
            };
            session.inner.epochs.validate(epoch.generation)?;
            Ok(Generated {
                generation: epoch.generation,
                value: EmojiPage {
                    emojis,
                    next_cursor,
                },
            })
        })
        .await
    }
}
