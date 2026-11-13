//! Revocable media capabilities and credential-separated fetching; immutable files and refs live below.

use super::{
    MediaData, MediaError, MediaSource,
    download::{denied, download},
    index::Reference,
};
use crate::{Generated, Session, generation::Epoch};
use std::sync::Arc;

/// Account-scoped encoded media service. No API accepts a local digest as proof of access.
#[derive(Clone)]
pub struct MediaService {
    session: Session,
}

impl Session {
    /// Gets this account's media capability. Existing handles are revoked on account replacement.
    #[must_use]
    pub fn media(&self) -> MediaService {
        MediaService {
            session: self.clone(),
        }
    }
}

impl MediaService {
    /// Reads only this account's previously authorized reference, without network access.
    /// # Errors
    /// Returns unavailable, damaged content/index, or revoked/stale session failures.
    pub async fn cached(&self, source: MediaSource) -> Result<Generated<MediaData>, MediaError> {
        self.run(move |session, epoch| async move {
            let store = &session.client.inner.media;
            let _gate = store.gate.lock().await;
            let reference = store
                .reference(session.account(), &source)
                .await?
                .ok_or(MediaError::Unavailable)?;
            let value = store.read_reference(reference).await?;
            session.inner.epochs.validate(epoch.generation)?;
            Ok(Generated {
                generation: epoch.generation,
                value,
            })
        })
        .await
    }

    /// Returns a scoped cached representation or fetches it. `refresh` revalidates mutable avatars;
    /// revision changes use a different key and always fetch unconditionally.
    /// # Errors
    /// Returns lifecycle, backend/provider, size, filesystem or rebuildable storage failures.
    pub async fn fetch(
        &self,
        source: MediaSource,
        refresh: bool,
    ) -> Result<Generated<MediaData>, MediaError> {
        self.run(move |session, epoch| async move {
            let store = &session.client.inner.media;
            let _admission = Arc::clone(&store.fetches)
                .acquire_owned()
                .await
                .map_err(|_| MediaError::TaskStopped)?;
            let previous = {
                let _gate = store.gate.lock().await;
                let mut reference = store.reference(session.account(), &source).await?;
                if let Some(known) = reference.clone() {
                    match store.read_reference(known).await {
                        Ok(value) if !refresh => {
                            session.inner.epochs.validate(epoch.generation)?;
                            return Ok(Generated {
                                generation: epoch.generation,
                                value,
                            });
                        }
                        Ok(_) => {}
                        Err(
                            MediaError::Unavailable
                            | MediaError::InvalidData
                            | MediaError::TooLarge,
                        ) => {
                            store.revoke(session.account(), &source).await?;
                            reference = None;
                        }
                        Err(error) => return Err(error),
                    }
                }
                reference
            };
            let mut response = match download(&session, &epoch, &source, previous.as_ref()).await {
                Ok(response) => response,
                Err(error) => {
                    if denied(&error) {
                        let _gate = store.gate.lock().await;
                        store.revoke_family(session.account(), &source).await?;
                    }
                    return Err(error);
                }
            };
            session.inner.epochs.validate(epoch.generation)?;
            let _gate = Arc::clone(&store.gate).lock_owned().await;
            if response.status == 304 {
                let reference = store
                    .reference(session.account(), &source)
                    .await?
                    .ok_or(MediaError::Unavailable)?;
                let value = store.read_reference(reference).await?;
                session.inner.epochs.validate(epoch.generation)?;
                return Ok(Generated {
                    generation: epoch.generation,
                    value,
                });
            }
            let range = super::ranges::normalize(&source, &mut response)?;
            let bytes: Arc<[u8]> = response.bytes.into();
            let digest = store.install(Arc::clone(&bytes)).await?;
            let text = |name| {
                response
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            let content_type = text(reqwest::header::CONTENT_TYPE);
            store
                .grant(
                    session.clone(),
                    epoch.generation,
                    source,
                    Reference {
                        digest,
                        content_type: content_type.clone(),
                        etag: text(reqwest::header::ETAG),
                        modified: text(reqwest::header::LAST_MODIFIED),
                        range,
                    },
                )
                .await?;
            session.inner.epochs.validate(epoch.generation)?;
            Ok(Generated {
                generation: epoch.generation,
                value: MediaData {
                    bytes,
                    content_type,
                    range,
                },
            })
        })
        .await
    }

    /// Collects unreferenced encoded files after the 24-hour recovery grace period.
    /// # Errors
    /// Returns revoked session or filesystem/index failures.
    pub async fn collect_orphans(&self) -> Result<(), MediaError> {
        self.run(|session, _epoch| async move {
            session
                .client
                .inner
                .media
                .collect(std::time::Duration::from_hours(24))
                .await
        })
        .await
    }

    /// Retires this account's encoded cache refs; other accounts' shared bytes remain protected.
    /// Credentials, durable drafts/operations, favorites and exported files are unaffected.
    /// # Errors
    /// Returns revoked session, index or filesystem failure. Subsequent fetches may refill the cache.
    pub async fn clear_cache(&self) -> Result<(), MediaError> {
        self.run(|session, _epoch| async move {
            session
                .client
                .inner
                .media
                .clear_refs(session.account())
                .await
        })
        .await
    }

    pub(super) async fn run<T, F, Fut>(&self, action: F) -> Result<T, MediaError>
    where
        T: Send + 'static,
        F: FnOnce(Session, Epoch) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, MediaError>> + Send + 'static,
    {
        let session = self.session.clone();
        let token = session.inner.tasks.token();
        self.session
            .client
            .dispatch(async move {
                let task = tokio::spawn(async move {
                    let _token = token;
                    let epoch = session.inner.epochs.current();
                    session.inner.epochs.validate(epoch.generation)?;
                    // An admitted file/index commit retains its task and gate if its caller is canceled.
                    action(session, epoch).await
                });
                Ok(task
                    .await
                    .map_err(|_| MediaError::TaskStopped)
                    .and_then(std::convert::identity))
            })
            .await?
    }
}
