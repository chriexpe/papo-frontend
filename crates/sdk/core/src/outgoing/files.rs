//! Durable account-owned attachment staging, streaming verification and reference-based collection; SQL acknowledgement lives in persistence.

mod verify;
pub(super) use verify::{open_for_upload, verify, verify_staged_metadata};

use super::{
    ClientError, OutgoingAttachment, OutgoingAttachmentContent, OutgoingError, StagedAttachment,
};
use crate::{AccountId, DataRoot, StorageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const MAX_FILE_BYTES: u64 = super::content::MAX_ATTACHMENT_BYTES;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Reference {
    pub(super) digest: [u8; 32],
    pub(super) size_bytes: u64,
}

pub(super) enum AttachmentInput {
    Inline(Vec<u8>),
    Staged(Reference),
}

pub(super) fn digest_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn ensure_account_directories(root: &Path, account: AccountId) -> Result<PathBuf, StorageError> {
    let directory = DataRoot::new(root).outgoing_files(account);
    for path in account_directories(root, &directory)? {
        crate::private_fs::staging_directory(&path)?;
    }
    Ok(directory)
}

fn account_directories(root: &Path, directory: &Path) -> Result<[PathBuf; 3], StorageError> {
    Ok([
        root.join("outgoing"),
        directory
            .parent()
            .ok_or(StorageError::InvalidState)?
            .to_path_buf(),
        directory.to_path_buf(),
    ])
}

#[cfg(unix)]
fn harden_account_directories(root: &Path, directory: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    for path in account_directories(root, directory)? {
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidState);
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Why a staging sink stopped accepting bytes; kept apart from `io::Error` so a transform's own
/// failure cannot hide a cancellation or the size ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SinkStop {
    Cancelled,
    TooLarge,
    Storage(ErrorKind),
}

/// The staged file being written: hashes every byte, enforces the 100 MiB ceiling and honours
/// cancellation on each write, so a transform streams into staging without a second copy.
pub(super) struct StagingSink<C: FnMut() -> bool, N: FnMut()> {
    file: std::io::BufWriter<File>,
    hasher: Sha256,
    total_bytes: u64,
    cancelled: C,
    copied_chunk: N,
    stop: Option<SinkStop>,
}

impl<C: FnMut() -> bool, N: FnMut()> Write for StagingSink<C, N> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(self.fail(SinkStop::Cancelled));
        }
        let added = u64::try_from(bytes.len()).map_err(|_| self.fail(SinkStop::TooLarge))?;
        let total = self
            .total_bytes
            .checked_add(added)
            .filter(|total| *total <= MAX_FILE_BYTES)
            .ok_or_else(|| self.fail(SinkStop::TooLarge))?;
        self.file
            .write_all(bytes)
            .map_err(|error| self.fail(SinkStop::Storage(error.kind())))?;
        self.hasher.update(bytes);
        self.total_bytes = total;
        (self.copied_chunk)();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<C: FnMut() -> bool, N: FnMut()> StagingSink<C, N> {
    fn fail(&mut self, stop: SinkStop) -> std::io::Error {
        self.stop = Some(stop);
        std::io::Error::other("staging sink stopped")
    }

    /// The sink's own failure, if it stopped the writer, as the typed outgoing error.
    fn stop_error(&self) -> Option<OutgoingError> {
        self.stop.map(|stop| match stop {
            SinkStop::Cancelled => ClientError::Closed.into(),
            SinkStop::TooLarge => OutgoingError::InvalidInput,
            SinkStop::Storage(kind) => {
                OutgoingError::Storage(StorageError::from(std::io::Error::from(kind)))
            }
        })
    }
}

/// Streams what `fill` writes into a private temporary file, hashing incrementally and enforcing
/// the 100 MiB ceiling during writing before syncing and atomically publishing `<sha256>`.
/// A sink failure (cancellation, ceiling, storage) wins over whatever error `fill` reports.
///
/// `publish` runs once the digest is known and before the blob appears under its name; what it
/// returns (a lock against collection, a pin) is held until the blob is in place and then handed
/// back, so collection can never see the new name unprotected.
pub(super) fn stage_stream<P>(
    root: &Path,
    account: AccountId,
    mut cancelled: impl FnMut() -> bool,
    copied_chunk: impl FnMut(),
    fill: impl FnOnce(&mut dyn Write) -> Result<(), OutgoingError>,
    publish: impl FnOnce(&str) -> P,
) -> Result<(Reference, P), OutgoingError> {
    if cancelled() {
        return Err(ClientError::Closed.into());
    }
    let directory = ensure_account_directories(root, account)?;
    let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp_path = directory.join(format!("{}{seq}", temp_prefix()));
    let mut temp_guard = TempFileGuard::new(temp_path.clone());
    let temp_file = crate::private_fs::new_staged_temp_file(&temp_path)?;

    let mut sink = StagingSink {
        file: std::io::BufWriter::with_capacity(COPY_BUFFER_BYTES, temp_file),
        hasher: Sha256::new(),
        total_bytes: 0,
        cancelled: &mut cancelled,
        copied_chunk,
        stop: None,
    };
    let filled = fill(&mut sink);
    if let Some(error) = sink.stop_error() {
        return Err(error);
    }
    filled?;
    let StagingSink {
        file: buffered,
        hasher,
        total_bytes,
        ..
    } = sink;
    let temp_file = buffered
        .into_inner()
        .map_err(|error| StorageError::from(error.into_error()))?;

    let digest: [u8; 32] = hasher.finalize().into();
    let name = digest_hex(&digest);
    let final_path = directory.join(&name);
    temp_file.sync_all().map_err(StorageError::from)?;
    let published = publish(&name);
    let reference = Reference {
        digest,
        size_bytes: total_bytes,
    };

    if existing_matches(&final_path, digest, total_bytes)? {
        drop(temp_file);
        // Drop temp_guard to unlink the duplicate temporary copy without touching final_path.
        drop(temp_guard);
        #[cfg(unix)]
        sync_directories(root, &directory)?;
        return Ok((reference, published));
    }

    drop(temp_file);
    #[cfg(windows)]
    if fs::symlink_metadata(&final_path).is_ok() {
        fs::remove_file(&final_path).map_err(StorageError::from)?;
    }
    fs::rename(&temp_path, &final_path).map_err(StorageError::from)?;
    temp_guard.disarm();
    #[cfg(unix)]
    sync_directories(root, &directory)?;
    Ok((reference, published))
}

/// Names this process gives its copies in progress; collection leaves them alone, while
/// leftovers of an earlier process (another id) are collected.
fn temp_prefix() -> String {
    format!(".tmp-{}-", std::process::id())
}

#[cfg(test)]
/// Streams a reader into staging through [`stage_stream`], with nothing to hold while publishing.
pub(super) fn stage_reader<R: Read>(
    root: &Path,
    account: AccountId,
    reader: R,
    cancelled: impl FnMut() -> bool,
    copied_chunk: impl FnMut(),
) -> Result<Reference, OutgoingError> {
    stage_reader_published(root, account, reader, cancelled, copied_chunk, |_| ())
        .map(|(reference, ())| reference)
}

/// Streams a reader into staging through [`stage_stream`], holding what `publish` returns.
pub(super) fn stage_reader_published<R: Read, P>(
    root: &Path,
    account: AccountId,
    mut reader: R,
    cancelled: impl FnMut() -> bool,
    copied_chunk: impl FnMut(),
    publish: impl FnOnce(&str) -> P,
) -> Result<(Reference, P), OutgoingError> {
    stage_stream(
        root,
        account,
        cancelled,
        copied_chunk,
        |sink| {
            let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
            loop {
                let read = loop {
                    match reader.read(&mut buffer) {
                        Ok(count) => break count,
                        Err(error) if error.kind() == ErrorKind::Interrupted => {}
                        Err(error) => return Err(OutgoingError::Source(error.kind())),
                    }
                };
                if read == 0 {
                    return Ok(());
                }
                sink.write_all(&buffer[..read])
                    .map_err(|_| OutgoingError::InvalidState)?;
            }
        },
        publish,
    )
}

fn existing_matches(path: &Path, digest: [u8; 32], size_bytes: u64) -> Result<bool, StorageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StorageError::InvalidState);
    }
    if metadata.len() != size_bytes {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(verify::hash_file(&mut file, &mut || false)? == Some((digest, size_bytes)))
}

#[cfg(unix)]
fn sync_directories(root: &Path, directory: &Path) -> Result<(), StorageError> {
    for path in [
        directory,
        directory.parent().ok_or(StorageError::InvalidState)?,
        directory
            .parent()
            .and_then(Path::parent)
            .ok_or(StorageError::InvalidState)?,
        root,
    ] {
        sync_directory(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

/// Detaches caller attachment payloads before serializing metadata into SQLite, enforcing
/// per-account ownership of staged references and keeping raw bytes out of SQLite rows.
pub(super) fn detach_inputs(
    files: &mut [OutgoingAttachment],
    account: AccountId,
) -> Result<Vec<AttachmentInput>, OutgoingError> {
    let mut inputs = Vec::with_capacity(files.len());
    for file in files {
        match file.take_content() {
            OutgoingAttachmentContent::Bytes(bytes) => {
                inputs.push(AttachmentInput::Inline(bytes));
            }
            OutgoingAttachmentContent::Staged(staged) => {
                if staged.account() != account {
                    return Err(OutgoingError::NotFound);
                }
                inputs.push(AttachmentInput::Staged(Reference {
                    digest: *staged.digest(),
                    size_bytes: staged.size_bytes(),
                }));
            }
            OutgoingAttachmentContent::Pending(_) | OutgoingAttachmentContent::Released => {
                return Err(OutgoingError::InvalidInput);
            }
        }
    }
    Ok(inputs)
}

/// Attaches durable staged references onto decoded attachment metadata without reading bytes.
pub(super) fn attach_references(
    account: AccountId,
    files: &mut [OutgoingAttachment],
    refs: &[Reference],
) -> Result<(), StorageError> {
    if files.len() != refs.len() {
        return Err(StorageError::InvalidState);
    }
    for (file, reference) in files.iter_mut().zip(refs) {
        if reference.size_bytes > MAX_FILE_BYTES {
            return Err(StorageError::InvalidState);
        }
        file.set_staged(StagedAttachment::new(
            account,
            reference.digest,
            reference.size_bytes,
        ));
    }
    Ok(())
}

pub(super) fn total_reference_bytes(refs: &[Reference]) -> Result<i64, StorageError> {
    refs.iter().try_fold(0_i64, |total, reference| {
        let len = i64::try_from(reference.size_bytes).map_err(|_| StorageError::InvalidState)?;
        total.checked_add(len).ok_or(StorageError::InvalidState)
    })
}

/// Called on the durable writer after committed reference changes, or before account recovery.
pub(super) fn referenced_names(
    account: AccountId,
    db: &rusqlite::Connection,
) -> Result<BTreeSet<String>, StorageError> {
    let mut names = BTreeSet::new();
    for table in ["operations", "drafts"] {
        let mut statement = db.prepare(&format!(
            "SELECT files FROM {table} WHERE server_id=?1 AND user_id=?2 AND files IS NOT NULL"
        ))?;
        let values = statement.query_map(
            rusqlite::params![account.server().as_bytes(), account.user().as_bytes()],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        for value in values {
            let refs: Vec<Reference> =
                serde_json::from_slice(&value?).map_err(|_| StorageError::InvalidState)?;
            names.extend(refs.iter().map(|reference| digest_hex(&reference.digest)));
        }
    }
    Ok(names)
}

pub(super) fn collect(
    root: &Path,
    account: AccountId,
    names: &BTreeSet<String>,
) -> Result<(), StorageError> {
    let directory = DataRoot::new(root).outgoing_files(account);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let in_progress = temp_prefix();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !names.contains(&name) && !name.starts_with(&in_progress) {
            fs::remove_file(entry.path())?;
        }
    }
    #[cfg(unix)]
    sync_directory(&directory)?;
    Ok(())
}

#[cfg(test)]
mod tests;
