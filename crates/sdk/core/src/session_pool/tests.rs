//! Session ownership wakeups preserve exact-account lookup and ignore stale retirement.

use super::*;
use crate::{
    AuthState, ServerAddress,
    secret::{CredentialGeneration, SecretKey},
};

fn session(pool: &SessionPool, user: u128) -> Arc<SessionInner> {
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), papo_api::Uuid::from_u128(user));
    SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::clone(&pool.next),
        pool.root.child_token(),
        AuthState::Ready,
    )
}

#[tokio::test(start_paused = true)]
async fn insertion_replacement_and_removal_wake_exact_account_lookup() {
    let pool = SessionPool::new();
    let mut changes = pool.subscribe_changes();
    let old = session(&pool, 1);
    let other = session(&pool, 2);
    assert!(!changes.has_changed().unwrap());
    pool.insert(Arc::clone(&old)).await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    pool.insert(Arc::clone(&other)).await;
    changes.borrow_and_update();
    let replacement = session(&pool, 1);
    pool.insert(Arc::clone(&replacement)).await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    pool.stop_if_current(&old).await;
    assert!(!changes.has_changed().unwrap());
    assert!(Arc::ptr_eq(&pool.get(old.account).unwrap(), &replacement));
    assert!(Arc::ptr_eq(&pool.get(other.account).unwrap(), &other));
    pool.stop_if_current(&replacement).await;
    assert!(changes.has_changed().unwrap());
    assert!(pool.get(old.account).is_none());
    assert!(pool.get(other.account).is_some());
    old.stop().await;
    pool.stop_all().await;
}

#[tokio::test(start_paused = true)]
async fn stop_and_shutdown_wake_even_when_registry_did_not_change() {
    let pool = SessionPool::new();
    let first = session(&pool, 1);
    pool.insert(Arc::clone(&first)).await;
    let mut changes = pool.subscribe_changes();
    pool.stop(first.account).await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    assert!(pool.get(first.account).is_none());
    pool.stop_all().await;
    assert!(changes.has_changed().unwrap());
    assert!(pool.root.is_cancelled());
}
