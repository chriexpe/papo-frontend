//! Streaming verification and upload opening for immutable account-owned staged files.

use super::{COPY_BUFFER_BYTES, MAX_FILE_BYTES, Reference, digest_hex};
use crate::{AccountId, DataRoot, OutgoingError, StorageError};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Seek, SeekFrom},
    path::Path,
};

/// Validates that a caller-referenced staged file still exists in this account's directory
/// with matching length and safe permissions before committing a durable SQLite reference.
pub(in crate::outgoing) fn verify_staged_metadata(
    root: &Path,
    account: AccountId,
    reference: Reference,
) -> Result<(), OutgoingError> {
    if reference.size_bytes > MAX_FILE_BYTES {
        return Err(OutgoingError::InvalidInput);
    }
    let directory = DataRoot::new(root).outgoing_files(account);
    #[cfg(unix)]
    if let Err(error) = super::harden_account_directories(root, &directory) {
        return match error {
            StorageError::Io(io) if io.kind() == ErrorKind::NotFound => {
                Err(OutgoingError::NotFound)
            }
            other => Err(other.into()),
        };
    }
    let path = directory.join(digest_hex(&reference.digest));
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(OutgoingError::NotFound),
        Err(error) => return Err(StorageError::from(error).into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StorageError::InvalidState.into());
    }
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(OutgoingError::NotFound),
        Err(error) => return Err(StorageError::from(error).into()),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(StorageError::from)?;
    }
    if file.metadata().map_err(StorageError::from)?.len() != reference.size_bytes {
        return Err(StorageError::InvalidState.into());
    }
    Ok(())
}

/// Streams each referenced staged file through a 64 KiB buffer to verify permissions, exact
/// length and SHA-256 without materializing file bytes in memory.
pub(in crate::outgoing) fn verify(
    root: &Path,
    account: AccountId,
    refs: &[Reference],
    mut cancelled: impl FnMut() -> bool,
) -> Result<(), OutgoingError> {
    if refs.is_empty() {
        return Ok(());
    }
    if cancelled() {
        return Err(crate::ClientError::Closed.into());
    }
    let directory = DataRoot::new(root).outgoing_files(account);
    #[cfg(unix)]
    super::harden_account_directories(root, &directory)?;
    for reference in refs {
        if cancelled() {
            return Err(crate::ClientError::Closed.into());
        }
        let mut source = open_validated_file(&directory, *reference)?;
        let Some((digest, size_bytes)) = hash_file(&mut source, &mut cancelled)? else {
            return Err(crate::ClientError::Closed.into());
        };
        if size_bytes != reference.size_bytes || digest != reference.digest {
            return Err(StorageError::InvalidState.into());
        }
    }
    Ok(())
}

/// Opens a validated, non-symlink staged file owned by `account` for incremental upload streaming.
pub(in crate::outgoing) fn open_for_upload(
    root: &Path,
    account: AccountId,
    reference: Reference,
) -> Result<File, StorageError> {
    let directory = DataRoot::new(root).outgoing_files(account);
    #[cfg(unix)]
    super::harden_account_directories(root, &directory)?;
    open_validated_file(&directory, reference)
}

fn open_validated_file(directory: &Path, reference: Reference) -> Result<File, StorageError> {
    if reference.size_bytes > MAX_FILE_BYTES {
        return Err(StorageError::InvalidState);
    }
    let path = directory.join(digest_hex(&reference.digest));
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StorageError::InvalidState);
    }
    let source = File::open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        source.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    if source.metadata()?.len() != reference.size_bytes {
        return Err(StorageError::InvalidState);
    }
    Ok(source)
}

pub(super) fn hash_file(
    file: &mut File,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<([u8; 32], u64)>, StorageError> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut bytes = vec![0_u8; COPY_BUFFER_BYTES];
    let mut total: u64 = 0;
    loop {
        if cancelled() {
            return Ok(None);
        }
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(count).map_err(|_| StorageError::InvalidState)?)
            .ok_or(StorageError::InvalidState)?;
        if total > MAX_FILE_BYTES {
            return Err(StorageError::InvalidState);
        }
        hash.update(&bytes[..count]);
    }
    Ok(Some((hash.finalize().into(), total)))
}
