//! Real allocated-byte accounting across SQLite sidecars and deduplicated encoded files.

use super::*;
use std::io::Write;

fn client(root: crate::DataRoot, budget_bytes: u64) -> crate::Client {
    crate::Client::open(
        root,
        Arc::new(crate::secret::test_store::ScriptedSecretStore::default()),
        crate::ClientConfig {
            disk: crate::DiskBudget {
                bytes: budget_bytes,
                maintenance_interval: Duration::from_hours(1),
            },
            ..crate::ClientConfig::default()
        },
    )
    .unwrap()
}

#[test]
fn held_reader_reports_pressure_and_later_trim_reclaims_without_new_victims() {
    let directory = tempfile::tempdir().unwrap();
    let root = crate::DataRoot::new(directory.path());
    let mut client = client(root.clone(), 2 * 1024 * 1024);
    let mut reader = rusqlite::Connection::open(root.state()).unwrap();
    let transaction = reader.transaction().unwrap();
    let _: i64 = transaction
        .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
        .unwrap();
    client
        .block_on(async {
            let registry = client.handle().inner.registry.clone();
            // Reuse the same pages twice while the reader pins the WAL prefix.
            // Truncation can then reclaim allocation without shrinking durable tables.
            for _ in 0..2 {
                registry
                    .writer
                    .write(|tx| -> Result<(), crate::StorageError> {
                        for index in 0..400 {
                            tx.execute(
                                "INSERT INTO settings(key,value) VALUES(?1,zeroblob(3000))",
                                [format!("fixture-{index}")],
                            )?;
                        }
                        Ok(())
                    })
                    .await
                    .unwrap();
                registry
                    .writer
                    .write(|tx| -> Result<(), crate::StorageError> {
                        tx.execute("DELETE FROM settings", [])?;
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            let report = client.handle().trim_disk().await.unwrap();
            assert!(report.pressure, "{report:?}");
            assert!(report.reclaim_blocked);
            assert_eq!(report.history_turns, 0);
            assert_eq!(report.media_evictions, 0);
        })
        .unwrap();
    transaction.commit().unwrap();
    client
        .block_on(async {
            let report = client.handle().trim_disk().await.unwrap();
            assert!(!report.pressure, "{report:?}");
            assert!(!report.reclaim_blocked);
            assert!(report.reclaimed_bytes > 0);
            assert_eq!(report.history_turns, 0);
            assert_eq!(report.media_evictions, 0);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn stale_victim_selection_cannot_recreate_a_removed_accounts_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = crate::DataRoot::new(directory.path());
    let mut client = client(root.clone(), 0);
    client
        .block_on(async {
            let handle = client.handle();
            let address = crate::storage_tests::address(1);
            let account = crate::storage_tests::account(&address, 1);
            let registry = handle.inner.registry.clone();
            registry
                .add_server(address, crate::UnixMicros(0))
                .await
                .unwrap();
            registry
                .add_account(account, None, None, false, crate::UnixMicros(0))
                .await
                .unwrap();
            let cache = handle.inner.caches.open(account).await.unwrap();
            let generation = crate::Generation(5);
            let channel = crate::Uuid::from_u128(5);
            let mut resources = crate::ResourceSnapshot::empty(generation);
            resources.channels = vec![crate::Channel {
                id: Some(channel.to_string()),
                ..crate::Channel::default()
            }]
            .into();
            cache.replace(&resources).await.unwrap();
            let set = cache
                .history_authorize(*channel.as_bytes(), generation, false)
                .await
                .unwrap();
            cache
                .history_commit(
                    *channel.as_bytes(),
                    set,
                    generation,
                    Vec::new(),
                    vec![crate::account_cache::history::HistoryRow {
                        table: crate::account_cache::history::HistoryTable::Messages,
                        id: *channel.as_bytes(),
                        payload: Vec::new(),
                    }],
                    Vec::new(),
                )
                .await
                .unwrap();
            let (candidates, _) = victims::select(&handle).await.unwrap();
            assert_eq!(candidates.len(), 1);
            handle.remove_account(account).await.unwrap();
            assert!(!root.account_cache(account).exists());
            let report = handle.disk_usage().await.unwrap();
            let result = reclamation::history(&handle, candidates, report)
                .await
                .unwrap();
            assert_eq!(result.turns, 0);
            assert!(!root.account_cache(account).exists());
            assert!(matches!(
                cache.load(generation).await,
                Err(crate::CacheError::Closed)
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn allocation_includes_database_wal_shm_and_counts_shared_bytes_once() {
    let directory = tempfile::tempdir().unwrap();
    let root = crate::DataRoot::new(directory.path());
    std::fs::create_dir_all(directory.path().join("media/aa")).unwrap();
    let paths = [
        root.state(),
        directory.path().join("state.sqlite-wal"),
        directory.path().join("state.sqlite-shm"),
        directory.path().join("media/index.sqlite"),
        directory.path().join("media/aa/shared"),
    ];
    let mut expected = 0;
    for path in &paths {
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&[7; 8193]).unwrap();
        file.sync_all().unwrap();
        expected += fs2::FileExt::allocated_size(&file).unwrap();
    }
    let report = measurement::measure(&root, expected).unwrap();
    assert_eq!(report.allocated_bytes, expected);
    assert!(!report.pressure);
    assert!(measurement::measure(&root, expected - 1).unwrap().pressure);
}

#[test]
fn under_budget_maintenance_collects_expired_orphans_but_preserves_recovery_grace() {
    let directory = tempfile::tempdir().unwrap();
    let root = crate::DataRoot::new(directory.path());
    let mut client = client(root.clone(), 1024 * 1024 * 1024);
    let modified = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let grace = Duration::from_hours(24);
    for (digest, timestamp) in [
        ([1; 32], modified),
        ([2; 32], modified + Duration::from_secs(1)),
    ] {
        let path = root.media_content(&digest);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [7; 8193]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
    }
    client
        .block_on(async {
            let handle = client.handle();
            handle
                .inner
                .media
                .writer
                .cache_write(|_| Ok::<_, MediaError>(()))
                .await
                .unwrap();
            assert!(!handle.disk_usage().await.unwrap().pressure);
            let report = trim_at(&handle, modified + grace).await.unwrap();
            assert!(!report.pressure);
            assert!(report.reclaimed_bytes > 0);
            assert!(!root.media_content(&[1; 32]).exists());
            assert!(root.media_content(&[2; 32]).exists());
        })
        .unwrap();
    client.shutdown().unwrap();
}
