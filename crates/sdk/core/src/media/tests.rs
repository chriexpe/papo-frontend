//! Reference isolation, shared content and deterministic orphan recovery; transport tests use loopback integration.

use super::*;
use crate::storage_tests::{Fixture, account, address};
use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime},
};

#[test]
fn digest_deduplication_never_manufactures_another_accounts_reference() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    let first = account(&address(1), 1);
    let other = account(&address(1), 2);
    let source = MediaSource::Attachment(crate::Uuid::from_u128(1));
    fixture.runtime().block_on(async {
        let digest = store
            .install(Arc::from(b"synthetic encoded bytes".as_slice()))
            .await
            .unwrap();
        assert_eq!(
            store
                .install(Arc::from(b"synthetic encoded bytes".as_slice()))
                .await
                .unwrap(),
            digest
        );
        let reference = index::Reference {
            digest,
            content_type: None,
            etag: None,
            modified: None,
            range: None,
        };
        let metadata = serde_json::to_vec(&reference).unwrap();
        let key = index::source_key(&source).unwrap();
        store
            .writer
            .cache_write(move |tx| -> Result<(), MediaError> {
                tx.execute("INSERT INTO files VALUES(?1,1)", [digest.as_slice()])?;
                tx.execute(
                    "INSERT INTO refs VALUES(?1,?2,?3,?4)",
                    rusqlite::params![index::account_key(first), key, digest.as_slice(), metadata],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(store.reference(first, &source).await.unwrap().is_some());
        assert!(store.reference(other, &source).await.unwrap().is_none());
        store.remove_account(other).await.unwrap();
        assert!(store.root.media_content(&digest).exists());
        store.remove_account(first).await.unwrap();
        assert!(!store.root.media_content(&digest).exists());
    });
}

#[test]
fn recreated_index_does_not_adopt_existing_content_and_orphans_observe_grace() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let digest = files::install(&root, b"orphan").unwrap();
    let path = root.media_content(&digest);
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    collection::collect_files(
        &root,
        &BTreeSet::new(),
        modified + Duration::from_secs(10),
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(path.exists());
    collection::collect_files(
        &root,
        &BTreeSet::new(),
        modified + Duration::from_secs(20),
        Duration::from_secs(20),
    )
    .unwrap();
    assert!(!path.exists());
}

#[test]
fn account_cleanup_preserves_fresh_unindexed_orphans_and_private_permissions() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    let digest = files::install(&fixture.root(), b"unindexed encoded fixture").unwrap();
    let path = fixture.root().media_content(&digest);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    fixture.runtime().block_on(async {
        store.remove_account(account(&address(1), 1)).await.unwrap();
    });
    assert!(path.exists());
}

#[test]
fn corrupted_reference_metadata_cannot_redirect_a_granted_digest() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    let owner = account(&address(1), 1);
    let source = MediaSource::Attachment(crate::Uuid::from_u128(2));
    fixture.runtime().block_on(async {
        let digest = store
            .install(Arc::from(b"authorized".as_slice()))
            .await
            .unwrap();
        let foreign = store
            .install(Arc::from(b"unrelated".as_slice()))
            .await
            .unwrap();
        let metadata = serde_json::to_vec(&index::Reference {
            digest: foreign,
            content_type: None,
            etag: None,
            modified: None,
            range: None,
        })
        .unwrap();
        let key = index::source_key(&source).unwrap();
        store
            .writer
            .cache_write(move |tx| -> Result<(), MediaError> {
                tx.execute("INSERT INTO files VALUES(?1,1)", [digest.as_slice()])?;
                tx.execute(
                    "INSERT INTO refs VALUES(?1,?2,?3,?4)",
                    rusqlite::params![index::account_key(owner), key, digest.as_slice(), metadata],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(matches!(
            store.reference(owner, &source).await,
            Err(MediaError::InvalidData)
        ));
    });
}

#[test]
fn global_media_eviction_uses_last_access_and_retires_all_shared_refs() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    fixture.runtime().block_on(async {
        let first = store.install(Arc::from(b"older".as_slice())).await.unwrap();
        let second = store.install(Arc::from(b"newer".as_slice())).await.unwrap();
        store
            .writer
            .cache_write(move |tx| -> Result<(), MediaError> {
                for (digest, order) in [(first, 1), (second, 2)] {
                    tx.execute(
                        "INSERT INTO files VALUES(?1,?2)",
                        rusqlite::params![digest.as_slice(), order],
                    )?;
                }
                for owner in [account(&address(1), 1), account(&address(1), 2)] {
                    tx.execute(
                        "INSERT INTO refs VALUES(?1,?2,?3,?4)",
                        rusqlite::params![
                            index::account_key(owner),
                            index::source_key(&MediaSource::ServerIcon)?,
                            second.as_slice(),
                            serde_json::to_vec(&index::Reference {
                                digest: second,
                                content_type: None,
                                etag: None,
                                modified: None,
                                range: None
                            })
                            .unwrap()
                        ],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        store.touch(first).await.unwrap();
        assert!(store.evict_one().await.unwrap().is_some());
        assert!(store.root.media_content(&first).exists());
        assert!(!store.root.media_content(&second).exists());
        for owner in [account(&address(1), 1), account(&address(1), 2)] {
            assert!(
                store
                    .reference(owner, &MediaSource::ServerIcon)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    });
}

#[test]
fn orphan_scan_is_bounded_and_resumes_through_a_large_prefix() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let mut paths = Vec::new();
    for index in 0_u128..250 {
        let mut digest = [0; 32];
        digest[16..].copy_from_slice(&index.to_be_bytes());
        let path = store.root.media_content(&digest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [7; 16]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(now))
            .unwrap();
        paths.push(path);
    }
    fixture.runtime().block_on(async {
        assert!(!store.collect_step(Duration::ZERO, now).await.unwrap());
        let removed = paths.iter().filter(|path| !path.exists()).count();
        assert!((1..=100).contains(&removed));
        let mut turns = 1;
        while !store.collect_step(Duration::ZERO, now).await.unwrap() {
            turns += 1;
            assert!(turns <= 4);
        }
        assert!(paths.iter().all(|path| !path.exists()));
    });
}

#[test]
fn explicit_collection_revisits_files_scanned_before_they_expired() {
    let fixture = Fixture::new();
    let store = MediaStore::new(fixture.storage()).unwrap();
    let created = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let grace = Duration::from_hours(24);
    let mut paths = Vec::new();
    for index in 0_u128..150 {
        let mut digest = [0; 32];
        digest[16..].copy_from_slice(&index.to_be_bytes());
        let path = store.root.media_content(&digest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [7; 16]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(created))
            .unwrap();
        paths.push(path);
    }
    fixture.runtime().block_on(async {
        assert!(!store.collect_step(grace, created).await.unwrap());
        assert!(paths.iter().all(|path| path.exists()));
        store.collect_at(grace, created + grace).await.unwrap();
        assert!(paths.iter().all(|path| !path.exists()));
    });
}
