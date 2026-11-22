//! Streaming temporary download staging and atomic promotion into `MediaStore`.
//!
//! Fresh downloads are authenticated by session/transport/range/byte-cap validation; SHA-256
//! provides content-addressed storage and post-install disk consistency, not backend media-ID
//! authentication. Callers exporting to user-visible paths must stage privately and rename only
//! after `MediaService` commits and returns `Ok`.

use super::{
    MediaError, MediaSource, MediaStore,
    download::{StreamBody, StreamedResponse},
    files::{COPY_BUFFER_BYTES, MAX_MEDIA_BYTES, unlink},
    pins::ReadPins,
    ranges::{StreamRange, finalize_stream_range, stream_plan},
};
use crate::{DataRoot, MediaRange};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

/// RAII owner of an unpublished download staging file inside `<root>/media/`.
pub(super) struct TempDownload {
    path: PathBuf,
    armed: bool,
}

impl TempDownload {
    fn create(root: &DataRoot) -> Result<(Self, File), MediaError> {
        let media_dir = root.path().join("media");
        crate::private_fs::staging_directory(&media_dir)?;
        let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = media_dir.join(format!("download-{pid}-{seq}.partial"));
        let file = crate::private_fs::new_staged_temp_file(&path)?;
        Ok((Self { path, armed: true }, file))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempDownload {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Completed download staged on disk before media-gate installation.
pub(super) struct StagedDownload {
    pub temp: TempDownload,
    pub digest: [u8; 32],
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub range: Option<MediaRange>,
}

struct ResponseHeaders {
    content_type: Option<String>,
    etag: Option<String>,
    modified: Option<String>,
}

impl ResponseHeaders {
    fn from_map(headers: &reqwest::header::HeaderMap) -> Self {
        let header = |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            content_type: header(reqwest::header::CONTENT_TYPE),
            etag: header(reqwest::header::ETAG),
            modified: header(reqwest::header::LAST_MODIFIED),
        }
    }
}

enum StageChunk {
    Data(bytes::Bytes),
    Finish,
}

/// Stages a streamed or inline response into a private temp file without accumulating `Vec<u8>`.
pub(super) async fn stage_response(
    store: &MediaStore,
    source: &MediaSource,
    response: StreamedResponse,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<StagedDownload, MediaError> {
    match response {
        StreamedResponse::Inline(mut inline) => {
            let range = super::ranges::normalize(source, &mut inline)?;
            let meta = ResponseHeaders::from_map(&inline.headers);
            let root = store.root.clone();
            let lease = Arc::clone(&store.lease);
            let bytes = inline.bytes;
            let (temp, digest) = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let (temp, mut file) = TempDownload::create(&root)?;
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                file.write_all(&bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|_| MediaError::Filesystem)?;
                Ok::<_, MediaError>((temp, digest))
            })
            .await
            .map_err(|_| MediaError::TaskStopped)??;
            Ok(StagedDownload {
                temp,
                digest,
                content_type: meta.content_type,
                etag: meta.etag,
                modified: meta.modified,
                range,
            })
        }
        StreamedResponse::Stream {
            status,
            headers,
            body,
        } => {
            let plan = stream_plan(source, status, &headers)?;
            let meta = ResponseHeaders::from_map(&headers);
            stage_body_stream(store, plan, *body, timeout, cancel, meta).await
        }
    }
}

async fn stage_body_stream(
    store: &MediaStore,
    plan: StreamRange,
    body: StreamBody,
    timeout: Duration,
    cancel: &CancellationToken,
    meta: ResponseHeaders,
) -> Result<StagedDownload, MediaError> {
    let (tx, mut rx) = mpsc::channel::<StageChunk>(4);
    let root = store.root.clone();
    let lease = Arc::clone(&store.lease);
    let slice_window = match plan {
        StreamRange::Slice {
            start_bytes,
            end_bytes,
        } => Some((start_bytes, end_bytes)),
        StreamRange::Full | StreamRange::Exact206(_) => None,
    };
    let writer_task = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let (temp, mut file) = TempDownload::create(&root)?;
        let mut hasher = Sha256::new();
        let mut received_total: u64 = 0;
        let mut written: u64 = 0;
        let mut finished = false;
        while let Some(item) = rx.blocking_recv() {
            match item {
                StageChunk::Data(chunk) => {
                    let chunk_len = chunk.len() as u64;
                    let chunk_start = received_total;
                    let chunk_end = received_total
                        .checked_add(chunk_len)
                        .ok_or(MediaError::TooLarge)?;
                    if chunk_end > MAX_MEDIA_BYTES {
                        return Err(MediaError::TooLarge);
                    }
                    received_total = chunk_end;
                    let slice = if let Some((start_bytes, end_bytes)) = slice_window {
                        let copy_start = chunk_start.max(start_bytes);
                        let copy_end_excl = end_bytes
                            .checked_add(1)
                            .map_or(chunk_end, |exclusive| chunk_end.min(exclusive));
                        if copy_start < copy_end_excl {
                            let from = usize::try_from(copy_start - chunk_start)
                                .map_err(|_| MediaError::InvalidData)?;
                            let to = usize::try_from(copy_end_excl - chunk_start)
                                .map_err(|_| MediaError::InvalidData)?;
                            &chunk[from..to]
                        } else {
                            &[]
                        }
                    } else {
                        chunk.as_ref()
                    };
                    if !slice.is_empty() {
                        hasher.update(slice);
                        file.write_all(slice).map_err(|_| MediaError::Filesystem)?;
                        written += slice.len() as u64;
                    }
                }
                StageChunk::Finish => {
                    finished = true;
                    break;
                }
            }
        }
        if !finished {
            return Err(MediaError::TaskStopped);
        }
        file.sync_all().map_err(|_| MediaError::Filesystem)?;
        let digest: [u8; 32] = hasher.finalize().into();
        Ok::<_, MediaError>((temp, digest, received_total, written))
    });

    let pump = pump_chunks(body, &tx, timeout, cancel).await;
    drop(tx);
    let writer_result = writer_task.await.map_err(|_| MediaError::TaskStopped)?;
    pump?;
    let (temp, digest, received_total, written) = writer_result?;
    let range = finalize_stream_range(plan, received_total, written)?;
    Ok(StagedDownload {
        temp,
        digest,
        content_type: meta.content_type,
        etag: meta.etag,
        modified: meta.modified,
        range,
    })
}

async fn pump_chunks(
    mut body: StreamBody,
    tx: &mpsc::Sender<StageChunk>,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        let next = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(crate::ClientError::Superseded.into()),
            () = &mut deadline => return Err(crate::ClientError::Timeout.into()),
            chunk = body.next_chunk() => chunk?,
        };
        if let Some(bytes) = next {
            if bytes.is_empty() {
                continue;
            }
            if tx.send(StageChunk::Data(bytes)).await.is_err() {
                return Ok(());
            }
        } else {
            let _ = tx.send(StageChunk::Finish).await;
            return Ok(());
        }
    }
}

/// Promotes a verified temporary file into content-addressed storage under `store.gate`.
pub(super) async fn install_temp_file(
    store: &MediaStore,
    temp: TempDownload,
    digest: [u8; 32],
) -> Result<[u8; 32], MediaError> {
    let root = store.root.clone();
    let lease = Arc::clone(&store.lease);
    let pins = store.pins.clone();
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        promote_temp(&root, temp, digest, &pins)
    })
    .await
    .map_err(|_| MediaError::TaskStopped)?
}

fn promote_temp(
    root: &DataRoot,
    mut temp: TempDownload,
    digest: [u8; 32],
    pins: &ReadPins,
) -> Result<[u8; 32], MediaError> {
    let target = root.media_content(&digest);
    let parent = target.parent().ok_or(MediaError::InvalidData)?;
    crate::private_fs::staging_directory(parent)?;
    if target.exists() {
        if files_equal_and_valid(&target, temp.path(), digest).is_ok_and(|equal| equal) {
            return Ok(digest);
        }
        if pins.contains(&digest) {
            return Err(MediaError::Unavailable);
        }
        unlink(&target)?;
    }
    fs::rename(temp.path(), &target).map_err(|_| MediaError::Filesystem)?;
    temp.disarm();
    Ok(digest)
}

fn files_equal_and_valid(
    existing: &Path,
    candidate: &Path,
    digest: [u8; 32],
) -> Result<bool, MediaError> {
    if fs::symlink_metadata(existing)
        .map_err(|_| MediaError::Unavailable)?
        .file_type()
        .is_symlink()
    {
        return Err(MediaError::InvalidData);
    }
    let meta_existing = fs::metadata(existing).map_err(|_| MediaError::Unavailable)?;
    let meta_candidate = fs::metadata(candidate).map_err(|_| MediaError::Unavailable)?;
    if meta_existing.len() != meta_candidate.len() || meta_existing.len() > MAX_MEDIA_BYTES {
        return Ok(false);
    }
    let mut a = File::open(existing).map_err(|_| MediaError::Unavailable)?;
    let mut b = File::open(candidate).map_err(|_| MediaError::Unavailable)?;
    let mut hasher = Sha256::new();
    let mut buf_a = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();
    let mut buf_b = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read_a = a.read(&mut buf_a).map_err(|_| MediaError::Unavailable)?;
        if read_a == 0 {
            break;
        }
        b.read_exact(&mut buf_b[..read_a])
            .map_err(|_| MediaError::Unavailable)?;
        if buf_a[..read_a] != buf_b[..read_a] {
            return Ok(false);
        }
        hasher.update(&buf_a[..read_a]);
    }
    Ok(<[u8; 32]>::from(hasher.finalize()) == digest)
}
