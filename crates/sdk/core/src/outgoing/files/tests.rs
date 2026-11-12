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
    collect(root.path(), a, &db).unwrap();
    let path = DataRoot::new(root.path());
    assert_eq!(fs::read_dir(path.outgoing_files(a)).unwrap().count(), 1);
    assert_eq!(fs::read_dir(path.outgoing_files(b)).unwrap().count(), 1);
}
