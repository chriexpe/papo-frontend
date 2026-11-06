//! Exclusive, bounded access to the desktop registry and its atomic replacement.
//!
//! A stable lock file is held for the profile lifetime and is never unlinked.
//! Only a missing registry means an empty profile. Every other read failure
//! prevents opening; no repair or reset overwrites unrecognized data. Callers
//! must run this synchronous API off GPUI's foreground and supply a trusted,
//! desktop-owned root when overriding the standard application data directory.

use std::{
    fs::{self, File, OpenOptions},
    io::Read as _,
    path::{Path, PathBuf},
};

use super::{Change, PersistenceError, REGISTRY_NAME, ServerList, atomic::atomic_write, model};

/// One MiB is the desktop registry's defensive on-disk byte limit, not a
/// message/cache budget. Reads take at most one extra byte to detect excess.
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;
/// A stable inode serializes cooperating processes across registry replacements.
const LOCK_NAME: &str = ".profile.lock";

/// An exclusively owned desktop profile. All methods may block on filesystem I/O.
///
/// Keep it on a worker. The lock is released on drop, including after failed
/// opening. A second process fails rather than risking lost updates. This is
/// coordination between cooperating clients, not a hostile same-user sandbox.
pub struct Profile {
    root: PathBuf,
    state: ServerList,
    _lock: File,
}

impl Profile {
    /// Resolves the settled `ProjectDirs("", "", "papo-desktop")` data root.
    /// Does not create or open any directory, and has no working-directory fallback.
    ///
    /// # Errors
    /// Returns `NoDataDirectory` when the OS cannot resolve a user data directory.
    pub fn data_directory() -> Result<PathBuf, PersistenceError> {
        directories::ProjectDirs::from("", "", "papo-desktop")
            .map(|directories| directories.data_dir().to_path_buf())
            .ok_or(PersistenceError::NoDataDirectory)
    }

    /// Opens a trusted, absolute desktop-owned root, creating it if missing. This never
    /// discovers or reads another client's files. Existing symlink roots and
    /// non-regular registry/lock entries are rejected. Ancestors are trusted.
    /// On Unix the root is private (`0700`) and new files are private (`0600`).
    /// Windows uses the user profile's inherited permissions; no secret is stored.
    ///
    /// # Errors
    /// Returns a sanitized filesystem/locking error, or a document validation
    /// error. Malformed and unsupported documents remain untouched.
    pub fn open(root: PathBuf) -> Result<Self, PersistenceError> {
        if !root.is_absolute() {
            return Err(PersistenceError::InvalidPath);
        }
        create_root(&root)?;
        let lock_path = root.join(LOCK_NAME);
        ordinary_file_or_missing(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options.open(lock_path)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(PersistenceError::Locked),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let state = load(&root.join(REGISTRY_NAME))?;
        Ok(Self {
            root,
            state,
            _lock: lock,
        })
    }

    /// The last committed list. After `DurabilityUnknown`, this includes the
    /// already-visible replacement, even though its directory sync failed.
    #[must_use]
    pub fn snapshot(&self) -> &ServerList {
        &self.state
    }

    /// Applies a non-secret list operation and persists it before returning success.
    /// Identical state does not rewrite the file. Removing a list row does not
    /// log out, delete credentials or clear core's cache; those need coordination.
    ///
    /// # Errors
    /// Returns validation or filesystem errors. Before replacement, failure keeps
    /// both the previous document and snapshot. `DurabilityUnknown` means the
    /// replacement occurred; the snapshot advances to match the visible document.
    pub fn apply(&mut self, change: Change) -> Result<ServerList, PersistenceError> {
        let next = self.state.apply(change)?;
        self.commit(next, atomic_write)
    }

    fn commit(
        &mut self,
        next: ServerList,
        write: impl FnOnce(&Path, &[u8]) -> Result<(), PersistenceError>,
    ) -> Result<ServerList, PersistenceError> {
        if next == self.state {
            return Ok(self.state.clone());
        }
        let bytes = model::encode(&next)?;
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(PersistenceError::TooLarge);
        }
        let result = write(&self.root, &bytes);
        if result.is_ok() || result == Err(PersistenceError::DurabilityUnknown) {
            self.state = next;
        }
        result.map(|()| self.state.clone())
    }
}

fn create_root(root: &Path) -> Result<(), PersistenceError> {
    #[cfg(unix)]
    let missing = missing_directories(root)?;
    if let Ok(metadata) = fs::symlink_metadata(root)
        && !metadata.is_dir()
    {
        return Err(PersistenceError::InvalidPath);
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        // Sync every newly created directory and its parent bottom-up. Syncing
        // only the registry's root would not persist that root's own new name.
        for directory in missing {
            File::open(&directory)?.sync_all()?;
            if let Some(parent) = directory.parent() {
                File::open(parent)?.sync_all()?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn missing_directories(root: &Path) -> Result<Vec<PathBuf>, PersistenceError> {
    let mut missing = Vec::new();
    let mut current = root;
    loop {
        match fs::symlink_metadata(current) {
            Ok(_) => return Ok(missing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                current = current.parent().ok_or(PersistenceError::InvalidPath)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn ordinary_file_or_missing(path: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(PersistenceError::InvalidPath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn load(path: &Path) -> Result<ServerList, PersistenceError> {
    ordinary_file_or_missing(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ServerList::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(PersistenceError::TooLarge);
    }
    model::decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_replacement_never_reaches_the_writer() {
        let directory = tempfile::tempdir().unwrap();
        let mut profile = Profile::open(directory.path().join("papo-desktop")).unwrap();
        let servers: Vec<_> = (0..129)
            .map(|index| {
                let prefix = format!("https://example.invalid/{index}/");
                format!("{prefix}{}", "a".repeat(8192 - prefix.len()))
            })
            .collect();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "servers": servers, "selected_server": null,
        }))
        .unwrap();
        let next = model::decode(&bytes).unwrap();
        let result = profile.commit(next, |_, _| panic!("oversized document reached the writer"));
        assert_eq!(result.err(), Some(PersistenceError::TooLarge));
        assert!(profile.snapshot().servers().is_empty());
        assert!(!profile.root.join(REGISTRY_NAME).exists());
    }

    #[test]
    fn rejected_commit_keeps_snapshot_and_document_and_can_retry() {
        let directory = tempfile::tempdir().unwrap();
        let mut profile = Profile::open(directory.path().join("papo-desktop")).unwrap();
        profile
            .apply(Change::Add {
                url: "https://first.example.invalid".into(),
            })
            .unwrap();
        let previous = fs::read(profile.root.join(REGISTRY_NAME)).unwrap();
        let next = profile
            .snapshot()
            .apply(Change::Add {
                url: "https://second.example.invalid".into(),
            })
            .unwrap();
        let failure = profile.commit(next.clone(), |_, _| {
            Err(PersistenceError::Io(std::io::ErrorKind::StorageFull))
        });
        assert_eq!(
            failure.err(),
            Some(PersistenceError::Io(std::io::ErrorKind::StorageFull))
        );
        assert_eq!(profile.snapshot().servers().len(), 1);
        assert_eq!(
            fs::read(profile.root.join(REGISTRY_NAME)).unwrap(),
            previous
        );
        assert_eq!(
            profile.commit(next, atomic_write).unwrap().servers().len(),
            2
        );
    }

    #[test]
    fn directory_sync_failure_advances_snapshot_to_visible_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let mut profile = Profile::open(directory.path().join("papo-desktop")).unwrap();
        let next = profile
            .snapshot()
            .apply(Change::Add {
                url: "https://first.example.invalid".into(),
            })
            .unwrap();
        let result = profile.commit(next, |root, bytes| {
            atomic_write(root, bytes)?;
            Err(PersistenceError::DurabilityUnknown)
        });
        assert_eq!(result.err(), Some(PersistenceError::DurabilityUnknown));
        assert_eq!(profile.snapshot().servers().len(), 1);
        assert!(profile.snapshot() == &load(&profile.root.join(REGISTRY_NAME)).unwrap());
    }
}
