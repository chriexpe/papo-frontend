//! Same-directory atomic replacement of this desktop client's registry.
//!
//! A temporary file is written and synced before replacing the destination.
//! Failures before replacement preserve the old document and drop the temporary
//! file. A later directory-sync failure means visible but uncertain durability.
//! Profile locking, root creation, byte limits and in-memory state belong to
//! the profile repository; callers run this blocking operation off the foreground.

use std::{fs, io, io::Write as _, path::Path};

use tempfile::NamedTempFile;

use super::{PersistenceError, REGISTRY_NAME};

/// Actual I/O boundaries of one registry replacement, not stage notifications.
/// Keeping these operations in the shared routine lets tests fail the operation
/// itself and detect a skipped sync, without abstracting unrelated filesystem work.
trait WriteIo {
    fn create(&self, root: &Path) -> io::Result<NamedTempFile>;
    fn write_all(&self, temporary: &mut NamedTempFile, bytes: &[u8]) -> io::Result<()>;
    fn sync_file(&self, temporary: &NamedTempFile) -> io::Result<()>;
    fn replace(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()>;
    fn sync_directory(&self, root: &Path) -> io::Result<()>;
}

struct FileIo;

impl WriteIo for FileIo {
    fn create(&self, root: &Path) -> io::Result<NamedTempFile> {
        NamedTempFile::new_in(root)
    }

    fn write_all(&self, temporary: &mut NamedTempFile, bytes: &[u8]) -> io::Result<()> {
        temporary.write_all(bytes)
    }

    fn sync_file(&self, temporary: &NamedTempFile) -> io::Result<()> {
        temporary.as_file().sync_all()
    }

    fn replace(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        temporary
            .persist(destination)
            .map(|_| ())
            .map_err(|error| error.error)
    }

    fn sync_directory(&self, root: &Path) -> io::Result<()> {
        sync_directory(root)
    }
}

/// Writes the fixed registry through a private, same-directory temporary file.
/// The caller must hold the profile lock and validate content and its size.
///
/// # Errors
///
/// Returns [`PersistenceError::InvalidPath`] for a non-regular destination and
/// sanitized [`PersistenceError::Io`] for any I/O failure before replacement.
/// [`PersistenceError::DurabilityUnknown`] means replacement succeeded but the
/// directory sync failed: callers must advance their snapshot to the visible data.
/// No failure before replacement leaves a temporary file intentionally retained.
pub(super) fn atomic_write(root: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    atomic_write_with(root, bytes, &FileIo)
}

fn atomic_write_with(root: &Path, bytes: &[u8], io: &impl WriteIo) -> Result<(), PersistenceError> {
    let destination = root.join(REGISTRY_NAME);
    ordinary_file_or_missing(&destination)?;
    let mut temporary = io.create(root)?;
    io.write_all(&mut temporary, bytes)?;
    io.sync_file(&temporary)?;
    io.replace(temporary, &destination)?;
    io.sync_directory(root)
        .map_err(|_| PersistenceError::DurabilityUnknown)
}

fn ordinary_file_or_missing(path: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(PersistenceError::InvalidPath),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Syncs a directory entry on Unix using a safe directory file handle.
/// On other platforms safe std APIs provide no equivalent, so this is a no-op;
/// file sync and replacement there do not claim power-loss directory durability.
///
/// # Errors
///
/// On Unix, returns the OS error if opening or syncing the directory fails.
/// The caller decides whether this failure occurred before or after a commit.
#[cfg_attr(
    not(unix),
    expect(
        clippy::unnecessary_wraps,
        reason = "Unix I/O errors require the same fallible boundary on every platform."
    )
)]
pub(super) fn sync_directory(root: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(root)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Ok(())
    }
}

#[cfg(test)]
#[path = "atomic_tests.rs"]
mod tests;
