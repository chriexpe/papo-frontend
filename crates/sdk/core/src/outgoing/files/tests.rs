//! Account isolation, immutable staged bytes and orphan cleanup without sockets or clocks.

use super::*;
use crate::{ServerAddress, Uuid};

fn account(user: u128) -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(user),
    )
}
#[test]
fn staged_file_is_immutable_verified_and_never_borrowed_from_another_account() {
    let root = tempfile::tempdir().unwrap();
    let a = account(1);
    let b = account(2);
    let refs = stage(root.path(), a, &[vec![0, 255, 1]]).unwrap();
    let mut files = vec![OutgoingAttachment {
        filename: "file.bin".into(),
        bytes: vec![],
    }];
    load(root.path(), a, &mut files, &refs).unwrap();
    assert_eq!(files[0].bytes, [0, 255, 1]);
    assert!(load(root.path(), b, &mut files, &refs).is_err());
    assert!(!DataRoot::new(root.path()).outgoing_files(b).exists());
    let path = DataRoot::new(root.path())
        .outgoing_files(a)
        .join(name(&refs[0].digest));
    fs::write(&path, [2]).unwrap();
    assert!(load(root.path(), a, &mut files, &refs).is_err());
    stage(root.path(), a, &[vec![0, 255, 1]]).unwrap();
    load(root.path(), a, &mut files, &refs).unwrap();
    assert_eq!(files[0].bytes, [0, 255, 1]);
}

#[test]
fn orphan_collection_preserves_referenced_files_and_other_accounts() {
    let root = tempfile::tempdir().unwrap();
    let a = account(1);
    let b = account(2);
    let refs = stage(root.path(), a, &[vec![0, 255, 1]]).unwrap();
    stage(root.path(), a, &[vec![2]]).unwrap();
    stage(root.path(), b, &[vec![3]]).unwrap();
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE operations(server_id BLOB,user_id BLOB,files BLOB);CREATE TABLE drafts(server_id BLOB,user_id BLOB,files BLOB);").unwrap();
    db.execute(
        "INSERT INTO operations VALUES(?1,?2,?3)",
        rusqlite::params![
            a.server().as_bytes(),
            a.user().as_bytes(),
            serde_json::to_vec(&refs).unwrap()
        ],
    )
    .unwrap();
    collect(root.path(), a, &referenced_names(a, &db).unwrap()).unwrap();
    let path = DataRoot::new(root.path());
    assert_eq!(fs::read_dir(path.outgoing_files(a)).unwrap().count(), 1);
    assert_eq!(fs::read_dir(path.outgoing_files(b)).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn staging_enforces_private_directories_and_files_below_a_public_root() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let account = account(1);
    let refs = stage(root.path(), account, &[vec![0, 255, 1]]).unwrap();
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
        fs::metadata(directory.join(name(&refs[0].digest)))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn loading_a_legacy_staged_file_hardens_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let account = account(1);
    let refs = stage(root.path(), account, &[vec![0, 255, 1]]).unwrap();
    let directory = DataRoot::new(root.path()).outgoing_files(account);
    let path = directory.join(name(&refs[0].digest));
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let mut values = vec![OutgoingAttachment {
        filename: "file.bin".into(),
        bytes: Vec::new(),
    }];
    load(root.path(), account, &mut values, &refs).unwrap();
    assert_eq!(values[0].bytes, [0, 255, 1]);
    assert_eq!(
        fs::metadata(directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
