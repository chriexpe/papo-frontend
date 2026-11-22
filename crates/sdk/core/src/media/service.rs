//! Revocable media capabilities and credential-separated fetching; immutable files and refs live below.

use super::{
    MediaData, MediaError, MediaMetadata, MediaSource,
    cached::CachedWrite,
    download::{denied, download, download_stream},
    index::Reference,
    stream_io::{install_temp_file, stage_response},
};
use crate::{Generated, Session, generation::Epoch};
use std::{io::Write, sync::Arc};

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
            let (_, value) = store
                .cached_reference(&session, &epoch, &source, false)
                .await?
                .ok_or(MediaError::Unavailable)?;
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
            let previous = match store
                .cached_reference(&session, &epoch, &source, true)
                .await?
            {
                Some((reference, value)) => {
                    if !refresh {
                        return Ok(Generated {
                            generation: epoch.generation,
                            value,
                        });
                    }
                    Some(reference)
                }
                None => None,
            };
            let mut response = match download(&session, &epoch, &source, previous.as_ref()).await {
                Ok(response) => response,
                Err(error) => {
                    if denied(&error) {
                        store
                            .revoke_denied(&session, epoch.generation, &source)
                            .await?;
                    }
                    return Err(error);
                }
            };
            session.inner.epochs.validate(epoch.generation)?;
            if response.status == 304 {
                let (_, value) = store
                    .cached_reference(&session, &epoch, &source, false)
                    .await?
                    .ok_or(MediaError::Unavailable)?;
                return Ok(Generated {
                    generation: epoch.generation,
                    value,
                });
            }
            let _gate = Arc::clone(&store.gate).lock_owned().await;
            session.inner.epochs.validate(epoch.generation)?;
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

    /// Streams this account's previously authorized cached representation into `writer` without
    /// buffering the whole object in memory.
    ///
    /// **Integrity & publication contract:** For cached reads, this verifies local on-disk
    /// integrity by checking SHA-256 against the previously committed `reference.digest` before
    /// writing any bytes to `writer`. Callers exporting to a user-visible path must still write
    /// to a private temporary file and only atomically rename/publish after `Ok(Generated<MediaMetadata>)`,
    /// because concurrent revocation or I/O failure can abort.
    ///
    /// # Errors
    /// Returns unavailable, damaged content/index, filesystem, or revoked/stale session failures.
    pub async fn cached_to_writer<W: Write + Send + 'static>(
        &self,
        source: MediaSource,
        writer: W,
    ) -> Result<Generated<MediaMetadata>, MediaError> {
        self.run(move |session, epoch| async move {
            let store = &session.client.inner.media;
            read_cached_hit(store, &session, &epoch, &source, writer).await
        })
        .await
    }

    /// Streams a scoped cached representation or downloads into temporary disk storage and copies
    /// the stored file to `writer` in bounded 64 KiB chunks.
    ///
    /// **Integrity & publication contract:**
    /// - **Cached hit:** verifies local on-disk integrity by checking SHA-256 against the
    ///   previously committed `reference.digest` before writing any bytes to `writer`.
    /// - **Newly fetched download:** authenticated by session/transport/range/byte-cap checks;
    ///   local SHA-256 serves content-addressing and post-install disk consistency, **not**
    ///   verification of the backend's opaque HMAC media ID.
    /// - **Caller publication:** Callers exporting to a user-visible path MUST write to a private
    ///   temporary file and atomically rename/publish only after `Ok(Generated<MediaMetadata>)`,
    ///   because concurrent revocation or I/O failure can abort mid-stream.
    ///
    /// When `refresh` is `true`, previously cached bytes are written only if revalidation returns
    /// HTTP 304; otherwise `writer` receives only the updated representation.
    ///
    /// # Errors
    /// Returns lifecycle, backend/provider, size, filesystem, or rebuildable storage failures.
    pub async fn fetch_to_writer<W: Write + Send + 'static>(
        &self,
        source: MediaSource,
        refresh: bool,
        mut writer: W,
    ) -> Result<Generated<MediaMetadata>, MediaError> {
        self.run(move |session, epoch| async move {
            let store = &session.client.inner.media;
            let _admission = Arc::clone(&store.fetches)
                .acquire_owned()
                .await
                .map_err(|_| MediaError::TaskStopped)?;
            let previous = if refresh {
                store
                    .verify_reference_metadata(&session, &epoch, &source, true)
                    .await?
            } else {
                match store
                    .cached_reference_to_writer(&session, &epoch, &source, true, writer)
                    .await?
                {
                    CachedWrite::Hit(reference, size_bytes) => {
                        return Ok(Generated {
                            generation: epoch.generation,
                            value: MediaMetadata {
                                content_type: reference.content_type,
                                range: reference.range,
                                size_bytes,
                            },
                        });
                    }
                    CachedWrite::Miss(returned_writer) => {
                        writer = returned_writer;
                        None
                    }
                }
            };
            let response = match download_stream(&session, &epoch, &source, previous.as_ref()).await
            {
                Ok(response) => response,
                Err(error) => {
                    if denied(&error) {
                        store
                            .revoke_denied(&session, epoch.generation, &source)
                            .await?;
                    }
                    return Err(error);
                }
            };
            session.inner.epochs.validate(epoch.generation)?;
            if response.status() == 304 {
                return read_cached_hit(store, &session, &epoch, &source, writer).await;
            }
            let _pin = stage_and_grant(&session, &epoch, &source, response).await?;
            read_cached_hit(store, &session, &epoch, &source, writer).await
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

impl super::MediaStore {
    pub(super) async fn revoke_denied(
        &self,
        session: &Session,
        generation: crate::Generation,
        source: &MediaSource,
    ) -> Result<(), MediaError> {
        let _gate = self.gate.lock().await;
        session.inner.epochs.validate(generation)?;
        self.revoke_family(session, generation, source).await
    }
}

/// Streams an already-granted and SHA-256-verified cached representation into `writer`.
async fn read_cached_hit<W: Write + Send + 'static>(
    store: &super::MediaStore,
    session: &Session,
    epoch: &Epoch,
    source: &MediaSource,
    writer: W,
) -> Result<Generated<MediaMetadata>, MediaError> {
    match store
        .cached_reference_to_writer(session, epoch, source, false, writer)
        .await?
    {
        CachedWrite::Hit(reference, size_bytes) => {
            session.inner.epochs.validate(epoch.generation)?;
            Ok(Generated {
                generation: epoch.generation,
                value: MediaMetadata {
                    content_type: reference.content_type,
                    range: reference.range,
                    size_bytes,
                },
            })
        }
        CachedWrite::Miss(_) => Err(MediaError::Unavailable),
    }
}

/// Stages a downloaded stream off-gate, promotes it atomically under the media gate, pins the
/// digest before dropping the gate, and commits the account reference grant.
async fn stage_and_grant(
    session: &Session,
    epoch: &Epoch,
    source: &MediaSource,
    response: super::download::StreamedResponse,
) -> Result<super::pins::ReadPin, MediaError> {
    let store = &session.client.inner.media;
    let staged = stage_response(
        store,
        source,
        response,
        session.client.inner.config.request_timeout,
        &epoch.cancel,
    )
    .await?;
    let gate = Arc::clone(&store.gate).lock_owned().await;
    session.inner.epochs.validate(epoch.generation)?;
    let digest = install_temp_file(store, staged.temp, staged.digest).await?;
    let pin = store.pins.pin(digest);
    store
        .grant(
            session.clone(),
            epoch.generation,
            source.clone(),
            Reference {
                digest,
                content_type: staged.content_type,
                etag: staged.etag,
                modified: staged.modified,
                range: staged.range,
            },
        )
        .await?;
    session.inner.epochs.validate(epoch.generation)?;
    drop(gate);
    Ok(pin)
}
