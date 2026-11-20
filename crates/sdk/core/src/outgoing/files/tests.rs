//! Account isolation, streaming staging, immutable blobs and orphan cleanup without sockets or clocks.

use super::*;
use crate::{ServerAddress, Uuid};
use std::io::{self, Cursor};

fn account(user: u128) -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(user),
    )
}

fn stage_bytes(root: &Path, acct: AccountId, bytes: &[u8]) -> Result<Reference, OutgoingError> {
    stage_reader(root, acct, Cursor::new(bytes), || false, || {})
}

struct RepeatingReader {
    remaining: u64,
    byte: u8,
}

impl Read for RepeatingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let n = usize::try_from(self.remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        buf[..n].fill(self.byte);
        self.remaining -= n as u64;
        Ok(n)
    }
}

struct FailingReader {
    sent: bool,
}

impl Read for FailingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.sent {
            self.sent = true;
            let n = buf.len().min(1024);
            buf[..n].fill(0xab);
            return Ok(n);
        }
        Err(io::Error::new(ErrorKind::BrokenPipe, "synthetic failure"))
    }
}

#[test]
fn staged_file_is_immutable_verified_and_never_borrowed_from_another_account() {
    let root = tempfile::tempdir().unwrap();
    let a = account(1);
    let b = account(2);
    let reference = stage_bytes(root.path(), a, &[0, 255, 1]).unwrap();
    verify(root.path(), a, &[reference], || false).unwrap();
    assert!(verify(root.path(), b, &[reference], || false).is_err());
    assert!(!DataRoot::new(root.path()).outgoing_files(b).exists());
    let path = DataRoot::new(root.path())
        .outgoing_files(a)
        .join(digest_hex(&reference.digest));
    assert_eq!(fs::read(&path).unwrap(), [0, 255, 1]);
    fs::write(&path, [2]).unwrap();
    assert!(verify(root.path(), a, &[reference], || false).is_err());
    stage_bytes(root.path(), a, &[0, 255, 1]).unwrap();
    verify(root.path(), a, &[reference], || false).unwrap();
    assert_eq!(fs::read(&path).unwrap(), [0, 255, 1]);
}

#[test]
fn streamed_staging_enforces_exact_100_mib_limit_and_cleans_up_partials() {
    let root = tempfile::tempdir().unwrap();
    let acct = account(7);
    let exact = stage_reader(
        root.path(),
        acct,
        RepeatingReader {
            remaining: MAX_FILE_BYTES,
            byte: 0x3c,
        },
        || false,
        || {},
    )
    .unwrap();
    assert_eq!(exact.size_bytes, MAX_FILE_BYTES);
    verify(root.path(), acct, &[exact], || false).unwrap();

    let over = stage_reader(
        root.path(),
        acct,
        RepeatingReader {
            remaining: MAX_FILE_BYTES + 1,
            byte: 0x4d,
        },
        || false,
        || {},
    );
    assert!(matches!(over, Err(OutgoingError::InvalidInput)));

    let failed = stage_reader(
        root.path(),
        acct,
        FailingReader { sent: false },
        || false,
        || {},
    );
    assert!(matches!(
        failed,
        Err(OutgoingError::Source(ErrorKind::BrokenPipe))
    ));

    // Only the single valid 100 MiB blob remains; no .tmp-* partial files leaked.
    let entries: Vec<_> = fs::read_dir(DataRoot::new(root.path()).outgoing_files(acct))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].file_name().to_string_lossy(),
        digest_hex(&exact.digest)
    );
}

#[test]
fn streamed_and_in_memory_staging_deduplicate_to_same_digest_and_bytes() {
    let root = tempfile::tempdir().unwrap();
    let acct = account(9);
    let payload: Vec<u8> = (0_usize..131_072)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let from_mem = stage_bytes(root.path(), acct, &payload).unwrap();
    let from_stream = stage_reader(
        root.path(),
        acct,
        io::BufReader::with_capacity(4096, Cursor::new(payload.clone())),
        || false,
        || {},
    )
    .unwrap();
    assert_eq!(from_mem, from_stream);
    let dir = DataRoot::new(root.path()).outgoing_files(acct);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    assert_eq!(
        fs::read(dir.join(digest_hex(&from_mem.digest))).unwrap(),
        payload
    );
}

#[test]
fn orphan_collection_preserves_referenced_files_reclaims_partials_and_isolates_accounts() {
    let root = tempfile::tempdir().unwrap();
    let a = account(1);
    let b = account(2);
    let reference = stage_bytes(root.path(), a, &[0, 255, 1]).unwrap();
    stage_bytes(root.path(), a, &[2]).unwrap();
    stage_bytes(root.path(), b, &[3]).unwrap();
    // Simulate a leftover partial temp file after a SIGKILL during staging.
    let partial = DataRoot::new(root.path())
        .outgoing_files(a)
        .join(".tmp-999-1");
    fs::write(&partial, b"partial-bytes").unwrap();

    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE operations(server_id BLOB,user_id BLOB,files BLOB);CREATE TABLE drafts(server_id BLOB,user_id BLOB,files BLOB);").unwrap();
    db.execute(
        "INSERT INTO operations VALUES(?1,?2,?3)",
        rusqlite::params![
            a.server().as_bytes(),
            a.user().as_bytes(),
            serde_json::to_vec(&[reference]).unwrap()
        ],
    )
    .unwrap();
    collect(root.path(), a, &referenced_names(a, &db).unwrap()).unwrap();
    let path = DataRoot::new(root.path());
    assert_eq!(fs::read_dir(path.outgoing_files(a)).unwrap().count(), 1);
    assert!(!partial.exists());
    assert_eq!(fs::read_dir(path.outgoing_files(b)).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn staging_enforces_private_directories_and_files_below_a_public_root() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let account = account(1);
    let reference = stage_bytes(root.path(), account, &[0, 255, 1]).unwrap();
    let directory = DataRoot::new(root.path()).outgoing_files(account);
    for path in [
        directory.clone(),
        directory.parent().unwrap().to_path_buf(),
        root.path().join("outgoing"),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        fs::metadata(directory.join(digest_hex(&reference.digest)))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn verifying_a_legacy_staged_file_hardens_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let account = account(1);
    let reference = stage_bytes(root.path(), account, &[0, 255, 1]).unwrap();
    let directory = DataRoot::new(root.path()).outgoing_files(account);
    let path = directory.join(digest_hex(&reference.digest));
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    verify(root.path(), account, &[reference], || false).unwrap();
    assert_eq!(
        fs::metadata(directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
