//! Off-Tokio private `.part` staging, fsync, and atomic rename for media file exports.
//!
//! Exports never publish partial bytes: failure, cancellation, or account/session closure removes
//! the staging file off Tokio worker threads without touching the destination path.

use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    media_disk_errors::FfiMediaError,
    media_records::{FfiMediaExportResult, FfiMediaSource},
};

/// Monotonic counter keeping sibling `.part` temporary filenames unique across concurrent exports.
static NEXT_EXPORT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

/// RAII owner of a private `.part` staging file deleted automatically unless atomically published.
struct PartFileGuard {
    path: PathBuf,
    disarmed: AtomicBool,
}

impl Drop for PartFileGuard {
    fn drop(&mut self) {
        if !self.disarmed.swap(true, Ordering::AcqRel) {
            let path = self.path.clone();
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                drop(rt.spawn_blocking(move || {
                    let _ = fs::remove_file(path);
                }));
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Removes the staging file synchronously when called from inside a blocking task.
fn remove_part_sync(guard: &PartFileGuard) {
    if !guard.disarmed.swap(true, Ordering::AcqRel) {
        let _ = fs::remove_file(&guard.path);
    }
}

/// Removes an unpublished `.part` file off Tokio workers when aborting or cancelling an export.
async fn cleanup_part_file(guard: &PartFileGuard) {
    if !guard.disarmed.swap(true, Ordering::AcqRel) {
        let path = guard.path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = fs::remove_file(path);
        })
        .await;
    }
}

/// Opens a unique private sibling `.part` file off Tokio workers after validating `destination`.
fn create_part_file(destination: &Path) -> Result<(PathBuf, File, File), FfiMediaError> {
    if destination.as_os_str().is_empty() || destination.is_dir() {
        return Err(FfiMediaError::InvalidPath);
    }
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.ok_or(FfiMediaError::InvalidPath)?;
    if !parent.is_dir() {
        return Err(FfiMediaError::InvalidPath);
    }
    let file_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(FfiMediaError::InvalidPath)?;
    let seq = NEXT_EXPORT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let part_path = parent.join(format!(".{file_name}.{pid}-{seq}.part"));
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts
        .open(&part_path)
        .map_err(|_| FfiMediaError::Filesystem)?;
    let sync_handle = file.try_clone().map_err(|_| {
        let _ = fs::remove_file(&part_path);
        FfiMediaError::Filesystem
    })?;
    Ok((part_path, file, sync_handle))
}

/// Flushes, syncs, and atomically renames the private `.part` file into `destination`, rechecking
/// shutdown and session closure immediately before rename so revocation never publishes bytes.
fn publish_part_file(
    sync_handle: File,
    guard: &PartFileGuard,
    destination: &Path,
    shutdown: &tokio_util::sync::CancellationToken,
    session: &papo_sdk::Session,
) -> Result<(), FfiMediaError> {
    if sync_handle.sync_all().is_err() {
        remove_part_sync(guard);
        return Err(FfiMediaError::Filesystem);
    }
    if shutdown.is_cancelled() || session.is_closed() {
        remove_part_sync(guard);
        return Err(FfiMediaError::Session {
            error: FfiClientError::Closed,
        });
    }
    if fs::rename(&guard.path, destination).is_err() {
        remove_part_sync(guard);
        return Err(FfiMediaError::Filesystem);
    }
    guard.disarmed.store(true, Ordering::Release);
    Ok(())
}

/// Exports media either via `cached_to_writer` or `fetch_to_writer` using atomic `.part` staging.
pub(crate) async fn export_to_file_impl(
    client: &FfiClient,
    session: papo_sdk::Session,
    source: FfiMediaSource,
    destination_path: String,
    refresh: bool,
    cached_only: bool,
) -> Result<FfiMediaExportResult, FfiMediaError> {
    let sdk_source = source.into_sdk()?;
    let dest = PathBuf::from(&destination_path);
    let dest_clone = dest.clone();
    let (part_path, writer, sync_handle) =
        tokio::task::spawn_blocking(move || create_part_file(&dest_clone))
            .await
            .map_err(|_| FfiMediaError::TaskStopped)??;
    let guard = Arc::new(PartFileGuard {
        path: part_path,
        disarmed: AtomicBool::new(false),
    });
    let service = session.media();
    let shutdown = client.shutdown.clone();
    let outcome = if cached_only {
        service.cached_to_writer(sdk_source, writer).await
    } else {
        service.fetch_to_writer(sdk_source, refresh, writer).await
    };
    let generated = match outcome {
        Ok(generated) => generated,
        Err(err) => {
            cleanup_part_file(&guard).await;
            return Err(err.into());
        }
    };
    if shutdown.is_cancelled() || session.is_closed() {
        cleanup_part_file(&guard).await;
        return Err(FfiMediaError::Session {
            error: FfiClientError::Closed,
        });
    }
    let guard_pub = Arc::clone(&guard);
    let shutdown_pub = shutdown.clone();
    let session_pub = session.clone();
    match tokio::task::spawn_blocking(move || {
        publish_part_file(sync_handle, &guard_pub, &dest, &shutdown_pub, &session_pub)
    })
    .await
    {
        Ok(Ok(())) => Ok(FfiMediaExportResult::from_generated(
            generated.generation,
            generated.value,
        )),
        Ok(Err(err)) => {
            cleanup_part_file(&guard).await;
            Err(err)
        }
        Err(_) => {
            cleanup_part_file(&guard).await;
            Err(FfiMediaError::TaskStopped)
        }
    }
}
