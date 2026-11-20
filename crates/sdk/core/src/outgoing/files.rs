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

/// Streams a reader into a private temporary file, hashing incrementally and enforcing the
/// 100 MiB ceiling during copying before syncing and atomically publishing `<sha256>`.
pub(super) fn stage_reader<R: Read>(
    root: &Path,
    account: AccountId,
    mut reader: R,
    mut cancelled: impl FnMut() -> bool,
    mut copied_chunk: impl FnMut(),
) -> Result<Reference, OutgoingError> {
    if cancelled() {
        return Err(ClientError::Closed.into());
    }
    let directory = ensure_account_directories(root, account)?;
    let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp_path = directory.join(format!(".tmp-{}-{seq}", std::process::id()));
    let mut temp_guard = TempFileGuard::new(temp_path.clone());
    let mut temp_file = crate::private_fs::new_staged_temp_file(&temp_path)?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut total_bytes: u64 = 0;

    loop {
        if cancelled() {
            return Err(ClientError::Closed.into());
        }
        let read = loop {
            match reader.read(&mut buffer) {
                Ok(count) => break count,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(OutgoingError::Source(error.kind())),
            }
        };
        if read == 0 {
            break;
        }
        total_bytes = total_bytes
            .checked_add(u64::try_from(read).map_err(|_| OutgoingError::InvalidInput)?)
            .ok_or(OutgoingError::InvalidInput)?;
        if total_bytes > MAX_FILE_BYTES {
            return Err(OutgoingError::InvalidInput);
        }
        temp_file
            .write_all(&buffer[..read])
            .map_err(StorageError::from)?;
        hasher.update(&buffer[..read]);
        copied_chunk();
    }

    let digest: [u8; 32] = hasher.finalize().into();
    let final_path = directory.join(digest_hex(&digest));

    if existing_matches(&final_path, digest, total_bytes)? {
        drop(temp_file);
        // Drop temp_guard to unlink the duplicate temporary copy without touching final_path.
        drop(temp_guard);
        #[cfg(unix)]
        sync_directories(root, &directory)?;
        return Ok(Reference {
            digest,
            size_bytes: total_bytes,
        });
    }

    temp_file.sync_all().map_err(StorageError::from)?;
    drop(temp_file);
    #[cfg(windows)]
    if fs::symlink_metadata(&final_path).is_ok() {
        fs::remove_file(&final_path).map_err(StorageError::from)?;
    }
    fs::rename(&temp_path, &final_path).map_err(StorageError::from)?;
    temp_guard.disarm();
    #[cfg(unix)]
    sync_directories(root, &directory)?;
    Ok(Reference {
        digest,
        size_bytes: total_bytes,
    })
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
            OutgoingAttachmentContent::Released => return Err(OutgoingError::InvalidInput),
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
    for entry in entries {
        let entry = entry?;
        if !names.contains(&entry.file_name().to_string_lossy().into_owned()) {
            fs::remove_file(entry.path())?;
        }
    }
    #[cfg(unix)]
    sync_directory(&directory)?;
    Ok(())
}

#[cfg(test)]
mod tests;
