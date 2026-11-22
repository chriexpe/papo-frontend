//! Account-authorized media fetching, bounded inline reads, and private `.part` file export.
//!
//! Cached reads verify local SHA-256 against the committed index before emitting bytes; fresh
//! fetches authenticate via session/transport/range/byte-cap checks. File creation, fsync, rename,
//! and failure cleanup stay strictly off Tokio worker threads.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, FfiHistoryCursor},
    media_disk_errors::FfiMediaError,
    media_part_file::export_to_file_impl,
    media_records::{
        BoundedVecWriter, FfiEmojiMetadata, FfiEmojiPage, FfiMediaData, FfiMediaExportResult,
        FfiMediaSource,
    },
};

#[uniffi::export]
impl FfiClient {
    /// Reads previously cached media up to 4 MiB directly without network access.
    ///
    /// # Errors
    /// Returns [`FfiMediaError::ExceedsInlineLimit`] if larger than 4 MiB, or [`FfiMediaError`].
    pub async fn cached_media_bytes(
        &self,
        account: FfiAccountId,
        source: FfiMediaSource,
    ) -> Result<FfiMediaData, FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        let sdk_source = source.into_sdk()?;
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
                let overflowed = Arc::new(AtomicBool::new(false));
                let sink = BoundedVecWriter {
                    buf: Arc::clone(&buf),
                    overflowed: Arc::clone(&overflowed),
                };
                match session.media().cached_to_writer(sdk_source, sink).await {
                    Ok(generated) => {
                        let bytes = std::mem::take(
                            &mut *buf
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                        );
                        Ok(FfiMediaData {
                            generation: generated.generation.get(),
                            bytes,
                            content_type: generated.value.content_type,
                            range: generated.value.range.map(Into::into),
                        })
                    }
                    Err(_) if overflowed.load(Ordering::Acquire) => {
                        Err(FfiMediaError::ExceedsInlineLimit)
                    }
                    Err(err) => Err(err.into()),
                }
            },
        )
        .await
    }

    /// Fetches or reads cached media up to 4 MiB directly into memory.
    ///
    /// # Errors
    /// Returns [`FfiMediaError::ExceedsInlineLimit`] if object exceeds 4 MiB; use `fetch_media_to_file`.
    pub async fn fetch_media_bytes(
        &self,
        account: FfiAccountId,
        source: FfiMediaSource,
        refresh: bool,
    ) -> Result<FfiMediaData, FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        let sdk_source = source.into_sdk()?;
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
                let overflowed = Arc::new(AtomicBool::new(false));
                let sink = BoundedVecWriter {
                    buf: Arc::clone(&buf),
                    overflowed: Arc::clone(&overflowed),
                };
                match session
                    .media()
                    .fetch_to_writer(sdk_source, refresh, sink)
                    .await
                {
                    Ok(generated) => {
                        let bytes = std::mem::take(
                            &mut *buf
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                        );
                        Ok(FfiMediaData {
                            generation: generated.generation.get(),
                            bytes,
                            content_type: generated.value.content_type,
                            range: generated.value.range.map(Into::into),
                        })
                    }
                    Err(_) if overflowed.load(Ordering::Acquire) => {
                        Err(FfiMediaError::ExceedsInlineLimit)
                    }
                    Err(err) => Err(err.into()),
                }
            },
        )
        .await
    }

    /// Exports cached media off Tokio workers into a private sibling `.part` file and atomically
    /// renames to `destination_path` only after full digest verification and commit succeed.
    ///
    /// # Errors
    /// Never publishes partial bytes; leaves `destination_path` untouched on error or revocation.
    pub async fn cached_media_to_file(
        self: &Arc<Self>,
        account: FfiAccountId,
        source: FfiMediaSource,
        destination_path: String,
    ) -> Result<FfiMediaExportResult, FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        let client = Arc::clone(self);
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                export_to_file_impl(&client, session, source, destination_path, false, true).await
            },
        )
        .await
    }

    /// Fetches (or serves cached) media off Tokio workers via private sibling `.part` staging and
    /// atomically publishes `destination_path` only after full verification and authorization commit.
    ///
    /// # Errors
    /// Never publishes partial bytes on network error, corrupt disk, cancellation, or revocation.
    pub async fn fetch_media_to_file(
        self: &Arc<Self>,
        account: FfiAccountId,
        source: FfiMediaSource,
        refresh: bool,
        destination_path: String,
    ) -> Result<FfiMediaExportResult, FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        let client = Arc::clone(self);
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                export_to_file_impl(&client, session, source, destination_path, refresh, false)
                    .await
            },
        )
        .await
    }

    /// Fetches one bounded page of custom server emojis and installs their image bytes into media cache.
    ///
    /// # Errors
    /// Returns [`FfiMediaError`].
    pub async fn fetch_emojis_page(
        &self,
        account: FfiAccountId,
        cursor: Option<FfiHistoryCursor>,
    ) -> Result<FfiEmojiPage, FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        let sdk_cursor = cursor.map(Into::into);
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let generated = session.media().emojis(sdk_cursor).await?;
                Ok(FfiEmojiPage {
                    generation: generated.generation.get(),
                    emojis: generated
                        .value
                        .emojis
                        .into_iter()
                        .map(|meta| FfiEmojiMetadata {
                            id: meta.id.to_string(),
                            name: meta.name,
                            format: meta.format,
                            created_at: meta.created_at,
                            source: FfiMediaSource::from_sdk(&meta.source),
                        })
                        .collect(),
                    next_cursor: generated.value.next_cursor.as_ref().map(Into::into),
                })
            },
        )
        .await
    }

    /// Retires this account's cached media reference grants without disturbing other accounts.
    ///
    /// # Errors
    /// Returns [`FfiMediaError`].
    pub async fn clear_media_cache(&self, account: FfiAccountId) -> Result<(), FfiMediaError> {
        let session = self
            .session_by_id(&account)
            .map_err(|error| FfiMediaError::Session { error })?;
        self.run_scoped(
            || FfiMediaError::Session {
                error: FfiClientError::Closed,
            },
            async move { session.media().clear_cache().await.map_err(Into::into) },
        )
        .await
    }
}
