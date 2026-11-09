//! Deterministic credential deadline, cancellation, and uncertain-outcome regressions.

use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use super::{
    test_store::{Behavior, Operation, ScriptedSecretStore},
    *,
};
use crate::ServerAddress;

pub(super) const DEADLINE: Duration = Duration::from_secs(5);
pub(super) const LATE: Duration = Duration::from_secs(20);

pub(super) fn key(generation: u64) -> SecretKey {
    let server = ServerAddress::parse("https://example.invalid")
        .unwrap()
        .id();
    SecretKey::new(
        AccountId::new(server, papo_api::Uuid::from_u128(1)),
        CredentialGeneration::new(generation),
    )
}

pub(super) fn value() -> SecretValue {
    SecretValue::new(vec![0])
}

pub(super) async fn ready() -> (
    Arc<ScriptedSecretStore>,
    Arc<SecretStoreBoundary>,
    SecretKey,
    CancellationToken,
) {
    let store = Arc::new(ScriptedSecretStore::default());
    let boundary = Arc::new(SecretStoreBoundary::new(store.clone(), DEADLINE));
    let key = key(1);
    let cancel = CancellationToken::new();
    boundary.activate(key.account(), key.generation()).unwrap();
    assert!(boundary.read(key, &cancel).await.unwrap().is_none());
    (store, boundary, key, cancel)
}

#[tokio::test(start_paused = true)]
async fn stalled_read_times_out_without_starving_runtime() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Read, Behavior::Stall);
    let started = tokio::time::Instant::now();
    let (result, ticker) = tokio::join!(boundary.read(key, &cancel), async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        1
    });
    assert!(matches!(
        result,
        Err(SecretError::Failure(SecretFailure::Timeout))
    ));
    assert_eq!(ticker, 1);
    assert_eq!(started.elapsed(), DEADLINE);
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Ready);
}

#[tokio::test(start_paused = true)]
async fn canceled_write_is_uncertain_and_next_use_reads_back() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Stall);
    let task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.write(key, value(), &cancel).await })
    };
    tokio::task::yield_now().await;
    cancel.cancel();
    assert_eq!(
        task.await.unwrap(),
        Err(SecretError::Uncertain(SecretFailure::Canceled))
    );
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Uncertain);
    let reads = store.calls(Operation::Read);
    assert!(
        boundary
            .read(key, &CancellationToken::new())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.calls(Operation::Read), reads + 1);
}

#[tokio::test(start_paused = true)]
async fn canceled_delete_cannot_be_reported_as_absent() {
    let (store, boundary, key, cancel) = ready().await;
    boundary.write(key, value(), &cancel).await.unwrap();
    store.push(Operation::Delete, Behavior::Stall);
    let task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.delete(key, &cancel).await })
    };
    tokio::task::yield_now().await;
    cancel.cancel();
    assert_eq!(
        task.await.unwrap(),
        Err(SecretError::Uncertain(SecretFailure::Canceled))
    );
    assert!(
        boundary
            .read(key, &CancellationToken::new())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test(start_paused = true)]
async fn late_write_requires_a_fence_even_when_read_observes_absence() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Detached(LATE));
    assert_eq!(
        boundary.write(key, value(), &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Timeout))
    );
    assert!(store.snapshot(key).is_none());
    assert!(matches!(
        boundary.read(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Unfenced))
    ));
    assert_eq!(
        boundary.delete(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Unfenced))
    );
    assert_eq!(store.calls(Operation::Delete), 0);
    tokio::time::advance(LATE).await;
    tokio::task::yield_now().await;
    assert_eq!(
        boundary.read(key, &cancel).await.unwrap().unwrap().expose(),
        [0]
    );
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Ready);
    boundary.delete(key, &cancel).await.unwrap();
    assert!(store.snapshot(key).is_none());
}

#[tokio::test(start_paused = true)]
async fn retry_of_late_delete_reads_back_and_acknowledges_only_fenced_absence() {
    let (store, boundary, key, cancel) = ready().await;
    boundary.write(key, value(), &cancel).await.unwrap();
    store.push(Operation::Delete, Behavior::Detached(LATE));
    assert_eq!(
        boundary.delete(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Timeout))
    );
    assert!(store.snapshot(key).is_some());
    assert_eq!(
        boundary.delete(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Unfenced))
    );
    tokio::time::advance(LATE).await;
    tokio::task::yield_now().await;
    let reads = store.calls(Operation::Read);
    boundary.delete(key, &cancel).await.unwrap();
    assert_eq!(store.calls(Operation::Read), reads + 1);
    assert_eq!(store.calls(Operation::Delete), 1);
    assert!(store.snapshot(key).is_none());
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Ready);
}

#[tokio::test(start_paused = true)]
async fn failed_read_back_prevents_next_mutation() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(
        Operation::Write,
        Behavior::Fail(SecretStoreError::AccessDenied),
    );
    assert_eq!(
        boundary.write(key, value(), &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Store(
            SecretStoreError::AccessDenied
        )))
    );
    store.push(
        Operation::Read,
        Behavior::Fail(SecretStoreError::Unavailable),
    );
    assert_eq!(
        boundary.write(key, value(), &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Store(
            SecretStoreError::Unavailable
        )))
    );
    assert_eq!(store.calls(Operation::Write), 1);
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Uncertain);
}

#[tokio::test(start_paused = true)]
async fn aborting_a_mutation_leaves_uncertainty_and_releases_admission() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Stall);
    let task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.write(key, value(), &cancel).await })
    };
    tokio::task::yield_now().await;
    assert_eq!(boundary.delete(key, &cancel).await, Err(SecretError::Busy));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Uncertain);
    boundary.delete(key, &cancel).await.unwrap();
    assert_eq!(store.calls(Operation::Delete), 0);
}

#[tokio::test(start_paused = true)]
async fn already_canceled_read_does_not_poll_the_host() {
    let (store, boundary, key, cancel) = ready().await;
    let reads = store.calls(Operation::Read);
    cancel.cancel();
    assert!(matches!(
        boundary.read(key, &cancel).await,
        Err(SecretError::Failure(SecretFailure::Canceled))
    ));
    assert_eq!(store.calls(Operation::Read), reads);
    assert_eq!(boundary.state(key).unwrap(), CredentialState::Ready);
}

#[tokio::test(start_paused = true)]
async fn reconciliation_itself_has_a_deadline() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Stall);
    let _ = boundary.write(key, value(), &cancel).await;
    store.push(Operation::Read, Behavior::Stall);
    assert_eq!(
        boundary.delete(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Timeout))
    );
    assert_eq!(store.calls(Operation::Delete), 0);
}
