//! Session lifecycle wake-ups are account-isolated and coalesce without timing or transport.

use super::*;
use crate::{
    AuthState, ServerAddress,
    secret::{CredentialGeneration, SecretKey},
};

fn session(user: u128) -> Arc<SessionInner> {
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), crate::Uuid::from_u128(user));
    SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::new(AtomicU64::new(1)),
        CancellationToken::new(),
        AuthState::Ready,
    )
}

#[tokio::test(start_paused = true)]
async fn insert_replacement_and_stop_wake_observers_without_crossing_accounts() {
    let pool = SessionPool::new();
    let mut changes = pool.subscribe();
    assert!(!changes.has_changed().unwrap());
    let first = session(1);
    let second = session(2);
    pool.insert(Arc::clone(&first)).await;
    pool.insert(Arc::clone(&second)).await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    let replacement = session(1);
    pool.insert(Arc::clone(&replacement)).await;
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    pool.stop_if_current(&first).await;
    assert!(!changes.has_changed().unwrap());
    assert!(Arc::ptr_eq(&pool.get(first.account).unwrap(), &replacement));
    pool.stop(first.account).await;
    assert!(changes.has_changed().unwrap());
    assert!(pool.get(first.account).is_none());
    assert!(Arc::ptr_eq(&pool.get(second.account).unwrap(), &second));
    changes.borrow_and_update();
    pool.stop_all().await;
    assert!(changes.has_changed().unwrap());
    assert!(pool.get(second.account).is_none());
}
