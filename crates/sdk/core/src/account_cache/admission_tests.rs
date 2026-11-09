//! Open-admission cancellation fences; accepted worker operations remain serialized with close and deletion.

use super::*;
use crate::{
    ServerAddress,
    storage_worker::{StorageWorker, WorkerConfig},
};
use papo_api::Uuid;
use std::{
    future::Future,
    sync::Mutex as BlockingMutex,
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

#[test]
fn canceled_open_before_lifecycle_admission_cannot_recreate_a_closed_cache() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = AccountCaches::new(
        root.clone(),
        Arc::new(worker.registrar()),
        Arc::new(ReadPool::default()),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let slot = caches.inner.slot(account()).await;
        let held = slot.cache.lock().await;
        {
            let opening = caches.open(account());
            tokio::pin!(opening);
            std::future::poll_fn(|context| {
                assert!(opening.as_mut().poll(context).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(
            Arc::strong_count(&caches.inner),
            1,
            "canceling pre-admission open left detached factory work"
        );
        drop(held);
        caches.close(account()).await.unwrap();
        assert!(!root.account_cache(account()).exists());
        assert!(slot.cache.lock().await.is_none());
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn canceled_admitted_open_finishes_installation_before_close_allows_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let register = worker.registrar();
    let (entered, initialized) = tokio::sync::oneshot::channel();
    let entered = Arc::new(BlockingMutex::new(Some(entered)));
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked = Arc::new(BlockingMutex::new(blocked));
    let caches = AccountCaches::new(
        root.clone(),
        Arc::new(move |path, initializer| {
            let entered = Arc::clone(&entered);
            let blocked = Arc::clone(&blocked);
            register(
                path,
                Arc::new(move |connection| {
                    initializer(connection)?;
                    if let Some(entered) = entered.lock().unwrap().take() {
                        entered.send(()).unwrap();
                        blocked.lock().unwrap().recv().unwrap();
                    }
                    Ok(())
                }),
            )
        }),
        Arc::new(ReadPool::default()),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let opening_caches = caches.clone();
        let opening = tokio::spawn(async move { opening_caches.open(account()).await });
        initialized.await.unwrap();
        opening.abort();
        assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
        let slot = caches.inner.slot(account()).await;
        assert!(
            slot.cache.try_lock().is_err(),
            "accepted open released its lifecycle fence"
        );
        let closing = caches.close(account());
        tokio::pin!(closing);
        std::future::poll_fn(|context| {
            assert!(closing.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        release.send(()).unwrap();
        closing.await.unwrap();
        assert!(slot.cache.lock().await.is_none());
        caches.invalidate(account()).await.unwrap();
        assert!(!root.account_cache(account()).exists());
        let reopened = caches.open(account()).await.unwrap();
        assert!(
            reopened
                .load(Generation(1))
                .await
                .unwrap()
                .channels
                .is_empty()
        );
        caches.close(account()).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}
