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

/// Shared SDK ownership, distinct from descriptors briefly inherited by fork/exec.
#[derive(Debug)]
pub(crate) struct ProfileLease(File);

impl Drop for ProfileLease {
    fn drop(&mut self) {
        // Closing alone leaves flock held while a forked child still has the descriptor.
        // All SDK work shares this Arc lease, so only its final owner releases the lock.
        let _ = self.0.unlock();
    }
}

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

    /// Durable, account-owned outgoing attachment staging; this is not the rebuildable media cache.
    #[must_use]
    pub fn outgoing_files(&self, account: AccountId) -> PathBuf {
        self.0
            .join("outgoing")
            .join(account.server().to_string())
            .join(account.user().to_string())
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

    pub(crate) fn acquire(&self) -> Result<Arc<ProfileLease>, StorageError> {
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
        let lease = Arc::new(ProfileLease(file));
        std::fs::create_dir_all(self.0.join("cache"))?;
        std::fs::create_dir_all(self.0.join("media"))?;
        Ok(lease)
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

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_descriptor_cannot_extend_the_final_sdk_lease() {
        let directory = tempfile::tempdir().unwrap();
        let root = DataRoot::new(directory.path());
        let lease = root.acquire().unwrap();
        // A forked child shares the open file description until its exec closes it.
        let inherited = lease.0.try_clone().unwrap();
        drop(lease);
        let next = root.acquire();
        assert!(
            next.is_ok(),
            "last SDK owner must explicitly unlock: {next:?}"
        );
        drop(inherited);
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
