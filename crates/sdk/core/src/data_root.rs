//! Data-root paths and process ownership; account authorization belongs to sessions.

use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{AccountId, StorageError};

/// Canonical SDK layout rooted in a client-selected directory.
#[derive(Clone, Debug)]
pub struct DataRoot(PathBuf);

impl DataRoot {
    /// Describes paths without creating or opening files.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    /// Client-selected root directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Durable database file.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.0.join("state.sqlite")
    }

    /// Rebuildable media index; this does not create its deferred schema.
    #[must_use]
    pub fn media_index(&self) -> PathBuf {
        self.0.join("media/index.sqlite")
    }

    /// Account-isolated cache path, not permission to open its contents.
    #[must_use]
    pub fn account_cache(&self, account: AccountId) -> PathBuf {
        self.0
            .join("cache")
            .join(account.server().to_string())
            .join(format!("{}.sqlite", account.user()))
    }

    /// Content-addressed media path for a SHA-256 digest.
    #[must_use]
    pub fn media_content(&self, digest: &[u8; 32]) -> PathBuf {
        use std::fmt::Write;
        let mut hex = String::with_capacity(64);
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        self.0.join("media").join(&hex[..2]).join(hex)
    }

    pub(crate) fn acquire(&self) -> Result<Arc<File>, StorageError> {
        std::fs::create_dir_all(&self.0)?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.0.join(".lock"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StorageError::ProfileInUse),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        std::fs::create_dir_all(self.0.join("cache"))?;
        std::fs::create_dir_all(self.0.join("media"))?;
        Ok(Arc::new(file))
    }
}

#[cfg(test)]
mod tests {
    //! Layout and lifetime tests; process-kill recovery lives in the Linux integration gate.
    use super::*;

    #[test]
    fn exclusive_lock_lasts_until_last_lease_is_released() {
        let directory = tempfile::tempdir().unwrap();
        let root = DataRoot::new(directory.path());
        let lease = root.acquire().unwrap();
        let other = Arc::clone(&lease);
        drop(lease);
        assert!(matches!(root.acquire(), Err(StorageError::ProfileInUse)));
        drop(other);
        assert!(root.acquire().is_ok());
    }

    #[test]
    fn root_setup_does_not_invent_cache_or_media_schemas() {
        let directory = tempfile::tempdir().unwrap();
        let root = DataRoot::new(directory.path());
        let _lease = root.acquire().unwrap();
        assert!(!root.media_index().exists());
        assert!(root.path().join("cache").is_dir());
        assert_eq!(
            root.media_content(&[0xab; 32]),
            root.path().join("media/ab").join("ab".repeat(32))
        );
    }
}
