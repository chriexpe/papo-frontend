//! Pins generation fencing, stale completion rejection, and host effects across boundary recreation.

use std::{sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use super::{
    test_store::{Behavior, Operation},
    tests::{DEADLINE, LATE, key, ready, value},
    *,
};

#[tokio::test(start_paused = true)]
async fn stale_write_completion_cannot_replace_new_generation_credentials() {
    let (store, boundary, old, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Delay(Duration::from_secs(1)));
    let task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.write(old, value(), &cancel).await })
    };
    tokio::task::yield_now().await;
    let new = key(2);
    boundary.activate(new.account(), new.generation()).unwrap();
    boundary
        .write(new, SecretValue::new(vec![1]), &cancel)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(task.await.unwrap(), Err(SecretError::Superseded));
    assert!(store.snapshot(old).is_some());
    assert_eq!(
        boundary.read(new, &cancel).await.unwrap().unwrap().expose(),
        [1]
    );
    assert_eq!(boundary.state(old), Err(SecretError::Superseded));
}

#[tokio::test(start_paused = true)]
async fn detached_old_delete_cannot_delete_replacement_generation() {
    let (store, boundary, old, cancel) = ready().await;
    boundary.write(old, value(), &cancel).await.unwrap();
    store.push(Operation::Delete, Behavior::Detached(LATE));
    assert_eq!(
        boundary.delete(old, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Timeout))
    );
    let new = key(2);
    boundary.activate(new.account(), new.generation()).unwrap();
    boundary
        .write(new, SecretValue::new(vec![1]), &cancel)
        .await
        .unwrap();
    tokio::time::advance(LATE).await;
    tokio::task::yield_now().await;
    assert!(store.snapshot(old).is_none());
    assert_eq!(
        boundary.read(new, &cancel).await.unwrap().unwrap().expose(),
        [1]
    );
    assert_eq!(boundary.state(new).unwrap(), CredentialState::Ready);
}

#[tokio::test(start_paused = true)]
async fn stale_read_never_exposes_old_generation_bytes() {
    let (store, boundary, old, cancel) = ready().await;
    boundary.write(old, value(), &cancel).await.unwrap();
    store.push(Operation::Read, Behavior::Delay(Duration::from_secs(1)));
    let task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.read(old, &cancel).await })
    };
    tokio::task::yield_now().await;
    let new = key(2);
    boundary.activate(new.account(), new.generation()).unwrap();
    assert!(boundary.read(new, &cancel).await.unwrap().is_none());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(task.await.unwrap(), Err(SecretError::Superseded)));
}

#[tokio::test(start_paused = true)]
async fn reactivation_does_not_reset_uncertainty_or_reuse_older_generations() {
    let (store, boundary, old, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Stall);
    let _ = boundary.write(old, value(), &cancel).await;
    boundary.activate(old.account(), old.generation()).unwrap();
    assert_eq!(boundary.state(old).unwrap(), CredentialState::Uncertain);
    let new = key(2);
    boundary.activate(new.account(), new.generation()).unwrap();
    assert_eq!(
        boundary.activate(old.account(), old.generation()),
        Err(SecretError::Superseded)
    );
}

#[tokio::test(start_paused = true)]
async fn recreated_boundary_cannot_clear_pending_old_os_effects() {
    let (store, boundary, key, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Detached(LATE));
    let _ = boundary.write(key, value(), &cancel).await;
    drop(boundary);
    let restarted = SecretStoreBoundary::new(store.clone(), DEADLINE);
    restarted.activate(key.account(), key.generation()).unwrap();
    assert_eq!(
        restarted.delete(key, &cancel).await,
        Err(SecretError::Uncertain(SecretFailure::Unfenced))
    );
    tokio::time::advance(LATE).await;
    tokio::task::yield_now().await;
    restarted.delete(key, &cancel).await.unwrap();
    assert!(store.snapshot(key).is_none());
    assert_eq!(store.calls(Operation::Delete), 1);
}

#[tokio::test(start_paused = true)]
async fn dropping_stale_call_does_not_release_new_generations_admission() {
    let (store, boundary, old, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Delay(LATE));
    let old_task = {
        let boundary = boundary.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.write(old, value(), &cancel).await })
    };
    tokio::task::yield_now().await;
    let new = key(2);
    boundary.activate(new.account(), new.generation()).unwrap();
    store.push(Operation::Read, Behavior::Stall);
    let new_task = {
        let boundary = boundary.clone();
        let cancel = CancellationToken::new();
        tokio::spawn(async move { boundary.read(new, &cancel).await })
    };
    tokio::task::yield_now().await;
    old_task.abort();
    assert!(old_task.await.unwrap_err().is_cancelled());
    assert_eq!(boundary.delete(new, &cancel).await, Err(SecretError::Busy));
    new_task.abort();
    assert!(new_task.await.is_err());
    assert_eq!(boundary.state(new).unwrap(), CredentialState::Uncertain);
}

#[tokio::test(start_paused = true)]
async fn one_accounts_stalled_call_does_not_block_another_account() {
    let (store, boundary, first, cancel) = ready().await;
    store.push(Operation::Write, Behavior::Stall);
    let task = {
        let boundary = Arc::clone(&boundary);
        let cancel = cancel.clone();
        tokio::spawn(async move { boundary.write(first, value(), &cancel).await })
    };
    tokio::task::yield_now().await;
    let second = SecretKey::new(
        AccountId::new(first.account().server(), papo_api::Uuid::from_u128(2)),
        first.generation(),
    );
    boundary
        .activate(second.account(), second.generation())
        .unwrap();
    boundary.write(second, value(), &cancel).await.unwrap();
    assert!(boundary.read(second, &cancel).await.unwrap().is_some());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}
