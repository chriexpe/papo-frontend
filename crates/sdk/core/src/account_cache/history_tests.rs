//! History storage invariants and crash-boundary fixtures; protocol behavior is tested in the model.

#[path = "history_callback_tests.rs"]
mod callbacks;
#[path = "history_fairness_tests.rs"]
mod fairness;
#[path = "history_recovery_tests.rs"]
mod recovery;
#[path = "history_visibility_tests.rs"]
mod visibility;

use super::{
    history::{HistoryKey, HistoryRow, HistoryTable},
    *,
};
use crate::{
    ServerAddress,
    storage_worker::{StorageWorker, WorkerConfig},
};
use papo_api::{
    Uuid,
    wire::{Channel, DirectConversation},
};
use rusqlite::{Connection, params};
use std::{
    future::Future,
    time::{Duration, Instant},
};

const CHANNEL: [u8; 16] = 5_u128.to_be_bytes();
const GENERATION: Generation = Generation(4);

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    )
}

fn snapshot() -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(GENERATION);
    snapshot.channels = vec![Channel {
        id: Some(Uuid::from_bytes(CHANNEL).to_string()),
        ..Channel::default()
    }]
    .into();
    snapshot
}

fn run<F: Future<Output = ()>>(test: impl FnOnce(AccountCaches, DataRoot) -> F) {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = AccountCaches::new(
        root.clone(),
        Arc::new(worker.registrar()),
        Arc::new(ReadPool::default()),
    );
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(test(caches, root));
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

fn row(id: u128) -> HistoryRow {
    HistoryRow {
        table: HistoryTable::Messages,
        id: id.to_be_bytes(),
        payload: Vec::new(),
    }
}

async fn populated(caches: &AccountCaches, count: u128) -> (Arc<AccountCache>, i64) {
    let cache = caches.open(account()).await.unwrap();
    cache.replace(&snapshot()).await.unwrap();
    let set_id = cache
        .history_authorize(CHANNEL, GENERATION, false)
        .await
        .unwrap();
    let rows: Vec<_> = (0..count).map(row).collect();
    for batch in rows.chunks(499) {
        cache
            .history_commit(
                CHANNEL,
                set_id,
                GENERATION,
                vec![1],
                batch.to_vec(),
                Vec::new(),
            )
            .await
            .unwrap();
    }
    (cache, set_id)
}

#[test]
fn version_one_migrates_forward_without_replacing_existing_rows() {
    let mut connection = Connection::open_in_memory().unwrap();
    schema::create_version_one(&mut connection, account()).unwrap();
    connection
        .execute(
            "INSERT INTO channels (revision,id,position,payload) VALUES (1,?1,0,X'')",
            [CHANNEL.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE cache_metadata SET channels_revision=1,next_revision=2,snapshot_version=7",
            [],
        )
        .unwrap();
    schema::initialize(&mut connection, account()).unwrap();
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        2
    );
    assert_eq!(schema::metadata(&connection, account()).unwrap().version, 7);
    assert_eq!(
        connection
            .query_row::<i64, _, _>(
                "SELECT count(*) FROM channels WHERE revision=1 AND id=?1",
                [CHANNEL.as_slice()],
                |row| row.get(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM history_sets", [], |row| row.get(0))
            .unwrap(),
        0
    );
}

#[test]
fn wrong_identity_is_rejected_before_version_one_migration() {
    let mut connection = Connection::open_in_memory().unwrap();
    schema::create_version_one(&mut connection, account()).unwrap();
    let other = AccountId::new(account().server(), Uuid::from_u128(2));
    assert_eq!(
        schema::initialize(&mut connection, other),
        Err(crate::storage_worker::WorkerError::IntegrityCheckFailed)
    );
    assert_eq!(
        connection
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>(
                "SELECT count(*) FROM sqlite_schema WHERE name='history_sets'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn incremental_tables_and_metadata_round_trip_without_snapshot_rewrites() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let rows = [
            HistoryTable::Chunks,
            HistoryTable::Items,
            HistoryTable::Messages,
            HistoryTable::Pins,
        ]
        .into_iter()
        .map(|table| HistoryRow {
            table,
            id: [9; 16],
            payload: vec![2],
        })
        .collect();
        cache
            .history_commit(
                CHANNEL,
                set_id,
                GENERATION,
                vec![3],
                rows,
                vec![HistoryKey {
                    table: HistoryTable::Messages,
                    id: [0; 16],
                }],
            )
            .await
            .unwrap();
        let data = cache
            .history_load(CHANNEL, GENERATION)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(data.metadata, vec![3]);
        assert_eq!(data.rows.len(), 4);
        assert!(data.rows.iter().all(|row| row.payload == [2]));
        assert!(matches!(
            cache
                .history_commit(
                    CHANNEL,
                    set_id,
                    GENERATION,
                    Vec::new(),
                    (0..500).map(row).collect(),
                    Vec::new()
                )
                .await,
            Err(CacheError::Storage(WorkerError::RowLimit))
        ));
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn marker_and_current_generation_filter_every_read_and_write() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let next = Generation(5);
        assert!(cache.history_load(CHANNEL, next).await.unwrap().is_none());
        assert_eq!(
            cache.history_authorize(CHANNEL, next, false).await.unwrap(),
            set_id
        );
        assert!(cache.history_load(CHANNEL, next).await.unwrap().is_some());
        cache.history_validate(CHANNEL, set_id, next).await.unwrap();
        cache
            .history_transaction(next, move |transaction| {
                transaction
                    .execute("UPDATE history_sets SET revoked=1 WHERE id=?1", [set_id])
                    .map_err(WorkerError::from)?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(cache.history_load(CHANNEL, next).await.unwrap().is_none());
        assert_eq!(
            cache.history_validate(CHANNEL, set_id, next).await,
            Err(CacheError::InvalidData)
        );
        assert_eq!(
            cache
                .history_commit(CHANNEL, set_id, next, Vec::new(), vec![row(1)], Vec::new())
                .await,
            Err(CacheError::InvalidData)
        );
        assert_eq!(
            cache.history_load(CHANNEL, GENERATION).await,
            Err(CacheError::Superseded)
        );
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn channel_and_dm_publication_preserves_survivors_and_revokes_disappearance() {
    run(|caches, _| async move {
        let (cache, old) = populated(&caches, 1).await;
        let mut updated = snapshot();
        updated.version = 1;
        updated.dms = vec![DirectConversation {
            id: Uuid::from_u128(8).to_string(),
            ..DirectConversation::default()
        }]
        .into();
        cache.update(&updated, &[CacheResource::Dms]).await.unwrap();
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .set_id,
            old
        );
        let dm = 8_u128.to_be_bytes();
        cache
            .history_authorize(dm, GENERATION, false)
            .await
            .unwrap();
        updated.channels = Vec::new().into();
        cache
            .update(&updated, &[CacheResource::Channels])
            .await
            .unwrap();
        assert!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        assert!(cache.history_load(dm, GENERATION).await.unwrap().is_some());
        updated.dms = Vec::new().into();
        cache.update(&updated, &[CacheResource::Dms]).await.unwrap();
        assert!(cache.history_load(dm, GENERATION).await.unwrap().is_none());
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn current_resource_rows_are_required_even_if_manifest_and_generation_match() {
    run(|caches, _| async move {
        let (cache, _) = populated(&caches, 1).await;
        cache
            .history_transaction(GENERATION, |transaction| {
                transaction
                    .execute(
                        "DELETE FROM channels WHERE id=?1",
                        params![CHANNEL.as_slice()],
                    )
                    .map_err(WorkerError::from)?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            cache.history_authorize(CHANNEL, GENERATION, false).await,
            Err(CacheError::InvalidData)
        );
        caches.close(account()).await.unwrap();
    });
}
