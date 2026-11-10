//! Cancellation and generation fences around actual cache operations, without time-based synchronization.

use super::*;
use crate::{
    ServerAddress,
    storage_worker::{SqlMutation, StorageWorker, WorkerConfig},
};
use papo_api::{
    Uuid,
    wire::{Channel, UserSummary},
};
use std::{
    future::Future,
    task::Poll,
    time::{Duration, Instant},
};

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    )
}

fn setup(directory: &tempfile::TempDir, worker: &StorageWorker) -> AccountCaches {
    AccountCaches::new(
        DataRoot::new(directory.path()),
        Arc::new(worker.registrar()),
        Arc::new(ReadPool::default()),
    )
}

#[test]
fn canceled_blocking_read_retains_cache_fence_until_the_query_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = setup(&directory, &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account()).await.unwrap();
        let context = cache.context();
        let fence = cache.fence().await.unwrap();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let read = tokio::spawn(async move {
            context
                .query(fence, move |_| {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(())
                })
                .await
        });
        started.await.unwrap();
        read.abort();
        assert!(read.await.unwrap_err().is_cancelled());
        assert!(
            cache.serial.try_lock().is_err(),
            "canceled future released a running blocking query"
        );
        let closing_caches = caches.clone();
        let close = tokio::spawn(async move { closing_caches.close(account()).await });
        assert!(!close.is_finished());
        release.send(()).unwrap();
        close.await.unwrap().unwrap();
        assert!(matches!(
            cache.load(Generation(1)).await,
            Err(CacheError::Closed)
        ));
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn newer_generation_claim_rejects_an_older_update_waiting_for_the_fence() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = setup(&directory, &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account()).await.unwrap();
        let held = cache.fence().await.unwrap();
        let old = ResourceSnapshot::empty(Generation(1));
        let update = cache.replace(&old);
        let load = cache.load(Generation(2));
        tokio::pin!(update, load);
        std::future::poll_fn(|context| {
            assert!(update.as_mut().poll(context).is_pending());
            assert!(load.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(held);
        assert!(matches!(update.await, Err(CacheError::Superseded)));
        assert!(load.await.unwrap().channels.is_empty());
        caches.close(account()).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn selective_updates_do_not_rewrite_large_unchanged_memberships() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = setup(&directory, &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account()).await.unwrap();
        let mut snapshot = ResourceSnapshot::empty(Generation(1));
        snapshot.members = (1..=1_501)
            .map(|id| UserSummary {
                id: Some(Uuid::from_u128(id).to_string()),
                ..UserSummary::default()
            })
            .collect::<Vec<_>>()
            .into();
        cache.replace(&snapshot).await.unwrap();
        let before: i64 = cache
            .readers
            .query(cache.path.clone(), |transaction| {
                transaction.query_row("SELECT members_revision FROM cache_metadata", [], |row| {
                    row.get(0)
                })
            })
            .await
            .unwrap();
        snapshot.version = 1;
        Arc::make_mut(&mut snapshot.channels).push(Channel {
            id: Some(Uuid::from_u128(99).to_string()),
            ..Channel::default()
        });
        cache
            .update(&snapshot, &[CacheResource::Channels])
            .await
            .unwrap();
        cache.update(&snapshot, &[]).await.unwrap();
        let (after, count): (i64, i64) = cache
            .readers
            .query(cache.path.clone(), |transaction| {
                transaction.query_row(
                    "SELECT members_revision,(SELECT count(*) FROM members) FROM cache_metadata",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(count, 1_501);
        assert_eq!(
            cache.load(Generation(1)).await.unwrap().members.len(),
            1_501
        );
        caches.close(account()).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn sparse_cached_positions_do_not_repeat_query_batches() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = setup(&directory, &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account()).await.unwrap();
        let mut snapshot = ResourceSnapshot::empty(Generation(1));
        snapshot.members = (1..=500)
            .map(|id| UserSummary {
                id: Some(Uuid::from_u128(id).to_string()),
                ..UserSummary::default()
            })
            .collect::<Vec<_>>()
            .into();
        cache.replace(&snapshot).await.unwrap();
        cache
            .writer
            .commit(
                SqlBatch::new(vec![SqlMutation::new(
                    "UPDATE members SET position=position+1000",
                    Vec::new(),
                )])
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cache.load(Generation(1)).await.unwrap().members.len(), 500);
        caches.close(account()).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn interrupted_staging_never_becomes_visible_and_is_pruned_after_next_publication() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = setup(&directory, &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account()).await.unwrap();
        let mut snapshot = ResourceSnapshot::empty(Generation(1));
        Arc::make_mut(&mut snapshot.members).push(UserSummary {
            id: Some(Uuid::from_u128(3).to_string()),
            ..UserSummary::default()
        });
        cache.replace(&snapshot).await.unwrap();
        cache
            .writer
            .commit(
                SqlBatch::new(vec![
                    SqlMutation::new(
                        "UPDATE cache_metadata SET next_revision=3 WHERE singleton=1",
                        Vec::new(),
                    ),
                    SqlMutation::new(
                        "INSERT INTO members (revision,id,position,payload) VALUES (2,?1,0,?2)",
                        vec![
                            rusqlite::types::Value::Blob(Uuid::from_u128(4).as_bytes().to_vec()),
                            rusqlite::types::Value::Blob(b"{}".to_vec()),
                        ],
                    ),
                ])
                .unwrap(),
            )
            .await
            .unwrap();
        caches.close(account()).await.unwrap();
        let cache = caches.open(account()).await.unwrap();
        let loaded = cache.load(Generation(2)).await.unwrap();
        assert_eq!(loaded.members.len(), 1);
        snapshot.generation = Generation(2);
        Arc::make_mut(&mut snapshot.members).clear();
        cache.replace(&snapshot).await.unwrap();
        let count: i64 = cache
            .readers
            .query(cache.path.clone(), |transaction| {
                transaction.query_row("SELECT count(*) FROM members", [], |row| row.get(0))
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
        caches.close(account()).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}
