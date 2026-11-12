//! Durable account-owned attachment staging and reference-based collection; SQL acknowledgement lives in persistence.

use super::OutgoingAttachment;
use crate::{AccountId, DataRoot, StorageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Reference {
    digest: [u8; 32],
    size_bytes: u64,
}
fn name(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut name = String::with_capacity(64);
    for byte in digest {
        let _ = write!(name, "{byte:02x}");
    }
    name
}

/// Called only on the durable writer, before its reference transaction commits.
pub(super) fn stage(
    root: &Path,
    account: AccountId,
    values: &[Vec<u8>],
) -> Result<Vec<Reference>, StorageError> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let directory = DataRoot::new(root).outgoing_files(account);
    fs::create_dir_all(&directory)?;
    let mut refs = Vec::with_capacity(values.len());
    for bytes in values {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let size_bytes = u64::try_from(bytes.len()).map_err(|_| StorageError::InvalidState)?;
        if size_bytes > MAX_FILE_BYTES {
            return Err(StorageError::InvalidState);
        }
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(name(&digest)))?;
        if file.metadata()?.len() != size_bytes || hash(&mut file)? != digest {
            // A process may have died while writing an unreferenced new digest. No valid
            // existing blob is rewritten; references become visible only after sync_all.
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(bytes)?;
        }
        file.sync_all()?;
        refs.push(Reference { digest, size_bytes });
    }
    // On Unix this also persists each new directory entry. Other platforms use the
    // staged file's sync_all boundary; the tested guarantee is process crash, not power loss.
    for path in [
        directory.as_path(),
        directory.parent().ok_or(StorageError::InvalidState)?,
        directory
            .parent()
            .and_then(Path::parent)
            .ok_or(StorageError::InvalidState)?,
        root,
    ] {
        sync_directory(path)?;
    }
    Ok(refs)
}

fn hash(file: &mut File) -> Result<[u8; 32], StorageError> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 16384];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(hash.finalize().into())
}
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

pub(super) fn detach(files: &mut [OutgoingAttachment]) -> Vec<Vec<u8>> {
    files
        .iter_mut()
        .map(|file| std::mem::take(&mut file.bytes))
        .collect()
}
pub(super) fn size(values: &[Vec<u8>]) -> Result<i64, StorageError> {
    values.iter().try_fold(0_i64, |total, bytes| {
        total
            .checked_add(i64::try_from(bytes.len()).map_err(|_| StorageError::InvalidState)?)
            .ok_or(StorageError::InvalidState)
    })
}

/// Called on the bounded storage read pool while the account's outgoing serial lock is held.
pub(super) fn load(
    root: &Path,
    account: AccountId,
    files: &mut [OutgoingAttachment],
    refs: &[Reference],
) -> Result<(), StorageError> {
    if files.len() != refs.len() {
        return Err(StorageError::InvalidState);
    }
    let directory = DataRoot::new(root).outgoing_files(account);
    for (file, reference) in files.iter_mut().zip(refs) {
        if reference.size_bytes > MAX_FILE_BYTES {
            return Err(StorageError::InvalidState);
        }
        let source = File::open(directory.join(name(&reference.digest)))?;
        if source.metadata()?.len() != reference.size_bytes {
            return Err(StorageError::InvalidState);
        }
        let mut bytes = Vec::new();
        source
            .take(reference.size_bytes + 1)
            .read_to_end(&mut bytes)?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if u64::try_from(bytes.len()).ok() != Some(reference.size_bytes)
            || digest != reference.digest
        {
            return Err(StorageError::InvalidState);
        }
        file.bytes = bytes;
    }
    Ok(())
}

/// Called on the durable writer after committed reference changes, or before account recovery.
/// Serializing it with file readers prevents unlinking a blob used by an in-flight draft load.
pub(crate) fn collect(
    root: &Path,
    account: AccountId,
    db: &rusqlite::Connection,
) -> Result<(), StorageError> {
    let directory = DataRoot::new(root).outgoing_files(account);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
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
            names.extend(refs.iter().map(|reference| name(&reference.digest)));
        }
    }
    for entry in entries {
        let entry = entry?;
        if !names.contains(&entry.file_name().to_string_lossy().into_owned()) {
            fs::remove_file(entry.path())?;
        }
    }
    sync_directory(&directory)
}

#[cfg(test)]
mod tests;
