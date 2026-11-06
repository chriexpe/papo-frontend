//! Atomic replacement failures at the production routine's actual I/O boundaries.
//!
//! Real temporary files prove pre-commit preservation and cleanup. Recorded calls
//! make skipping a sync or replacement fail even when the remaining writes work.
//! The bytes are synthetic; these tests never open a user's profile.

use std::{cell::RefCell, fs, io, path::Path};

use tempfile::NamedTempFile;

use super::{FileIo, PersistenceError, WriteIo, atomic_write, atomic_write_with, sync_directory};

/// Synthetic previous content; the atomic layer does not interpret the schema.
const PREVIOUS: &[u8] = b"previous synthetic registry";
/// Synthetic replacement content, distinct from the previous document.
const REPLACEMENT: &[u8] = b"replacement synthetic registry";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Create,
    Write,
    FileSync,
    Replace,
    DirectorySync,
}

/// Required order of operations in the shared production commit routine.
const ORDER: [Boundary; 5] = [
    Boundary::Create,
    Boundary::Write,
    Boundary::FileSync,
    Boundary::Replace,
    Boundary::DirectorySync,
];

struct FaultIo {
    failure: Option<Boundary>,
    calls: RefCell<Vec<Boundary>>,
}

impl FaultIo {
    fn new(failure: Option<Boundary>) -> Self {
        Self {
            failure,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn boundary(&self, boundary: Boundary) -> io::Result<()> {
        self.calls.borrow_mut().push(boundary);
        if self.failure == Some(boundary) {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "synthetic failure",
            ));
        }
        Ok(())
    }
}

impl WriteIo for FaultIo {
    fn create(&self, root: &Path) -> io::Result<NamedTempFile> {
        self.boundary(Boundary::Create)?;
        let temporary = FileIo.create(root)?;
        assert_eq!(temporary.path().parent(), Some(root));
        Ok(temporary)
    }

    fn write_all(&self, temporary: &mut NamedTempFile, bytes: &[u8]) -> io::Result<()> {
        if self.failure == Some(Boundary::Write) {
            // A real partial write must be cleaned up just like a zero-byte error.
            FileIo.write_all(temporary, &bytes[..bytes.len() / 2])?;
        }
        self.boundary(Boundary::Write)?;
        FileIo.write_all(temporary, bytes)
    }

    fn sync_file(&self, temporary: &NamedTempFile) -> io::Result<()> {
        self.boundary(Boundary::FileSync)?;
        FileIo.sync_file(temporary)
    }

    fn replace(&self, temporary: NamedTempFile, destination: &Path) -> io::Result<()> {
        self.boundary(Boundary::Replace)?;
        FileIo.replace(temporary, destination)
    }

    fn sync_directory(&self, root: &Path) -> io::Result<()> {
        self.boundary(Boundary::DirectorySync)?;
        FileIo.sync_directory(root)
    }
}

#[test]
fn successful_write_uses_every_boundary_once_in_durability_order() {
    let directory = tempfile::tempdir().unwrap();
    let io = FaultIo::new(None);
    atomic_write_with(directory.path(), REPLACEMENT, &io).unwrap();
    assert_eq!(*io.calls.borrow(), ORDER);
    assert_eq!(
        fs::read(directory.path().join("servers.json")).unwrap(),
        REPLACEMENT
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn production_wrapper_creates_and_replaces_only_the_fixed_registry() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("unrelated"), PREVIOUS).unwrap();
    atomic_write(directory.path(), PREVIOUS).unwrap();
    atomic_write(directory.path(), REPLACEMENT).unwrap();
    assert_eq!(
        fs::read(directory.path().join("servers.json")).unwrap(),
        REPLACEMENT
    );
    assert_eq!(
        fs::read(directory.path().join("unrelated")).unwrap(),
        PREVIOUS
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn every_pre_commit_io_failure_preserves_old_bytes_and_cleans_temporary_file() {
    for (index, failure) in ORDER[..4].iter().copied().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("servers.json");
        fs::write(&path, PREVIOUS).unwrap();
        let io = FaultIo::new(Some(failure));
        assert_eq!(
            atomic_write_with(directory.path(), REPLACEMENT, &io),
            Err(PersistenceError::Io(io::ErrorKind::StorageFull))
        );
        assert_eq!(*io.calls.borrow(), ORDER[..=index]);
        assert_eq!(fs::read(&path).unwrap(), PREVIOUS);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        atomic_write(directory.path(), REPLACEMENT).unwrap();
        assert_eq!(fs::read(path).unwrap(), REPLACEMENT);
    }
}

#[test]
fn every_pre_commit_io_failure_keeps_a_missing_registry_missing() {
    for failure in ORDER[..4].iter().copied() {
        let directory = tempfile::tempdir().unwrap();
        let io = FaultIo::new(Some(failure));
        assert_eq!(
            atomic_write_with(directory.path(), REPLACEMENT, &io),
            Err(PersistenceError::Io(io::ErrorKind::StorageFull))
        );
        assert!(!directory.path().join("servers.json").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn directory_sync_failure_reports_uncertain_durability_after_visible_commit() {
    for previous in [None, Some(PREVIOUS)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("servers.json");
        if let Some(bytes) = previous {
            fs::write(&path, bytes).unwrap();
        }
        let io = FaultIo::new(Some(Boundary::DirectorySync));
        assert_eq!(
            atomic_write_with(directory.path(), REPLACEMENT, &io),
            Err(PersistenceError::DurabilityUnknown)
        );
        assert_eq!(*io.calls.borrow(), ORDER);
        assert_eq!(fs::read(path).unwrap(), REPLACEMENT);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn non_regular_destination_is_rejected_before_creating_a_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("servers.json")).unwrap();
    let io = FaultIo::new(None);
    assert_eq!(
        atomic_write_with(directory.path(), REPLACEMENT, &io),
        Err(PersistenceError::InvalidPath)
    );
    assert!(io.calls.borrow().is_empty());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symlink_destination_does_not_overwrite_its_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("unrelated");
    fs::write(&other, PREVIOUS).unwrap();
    symlink(&other, directory.path().join("servers.json")).unwrap();
    let io = FaultIo::new(None);
    assert_eq!(
        atomic_write_with(directory.path(), REPLACEMENT, &io),
        Err(PersistenceError::InvalidPath)
    );
    assert!(io.calls.borrow().is_empty());
    assert_eq!(fs::read(other).unwrap(), PREVIOUS);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn unix_directory_sync_propagates_a_real_open_failure() {
    let directory = tempfile::tempdir().unwrap();
    let error = sync_directory(&directory.path().join("missing")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    sync_directory(directory.path()).unwrap();
}

#[cfg(not(unix))]
#[test]
fn non_unix_directory_sync_is_explicitly_a_no_op() {
    let directory = tempfile::tempdir().unwrap();
    sync_directory(&directory.path().join("missing")).unwrap();
}
