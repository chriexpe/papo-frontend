//! A loopback integration regression for actor drain and publication bounds.
//!
//! The backend uses ephemeral data only. Acknowledged Wake calls establish a
//! full Net queue before the actor starts draining; this does not rely on sleeps.

#[path = "../support/fixture_backend.rs"]
mod fixture_backend;

use super::super::{SessionIdentity, actor::SessionThread};
use fixture_backend::{ACCOUNT_ID, CHANNEL_ID, FixtureBackend, SESSION_TOKEN};
use papo_core::{
    api::net::Wake,
    cache::ClientDb,
    notification::NotificationCoordinator,
    runtime::{DEFAULT_DRAIN_LIMIT, ServerRuntime},
    state::Store,
    storage::{MemorySecretStore, Secret, SecretStore as _},
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

#[test]
fn replay_drains_are_bounded_and_a_full_foreground_queue_cannot_strand_shutdown() {
    let backend = FixtureBackend::start(10_000, 2_000).unwrap();
    let url = backend.url().to_owned();
    let identity =
        SessionIdentity::new(papo_core::server_key(&url).into(), Some(ACCOUNT_ID.into()));
    let owner = SessionThread::start(identity, move |_wake| {
        let (published, ready) = mpsc::channel();
        let wake = Wake::new(move || {
            let _ = published.send(());
        });
        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
        let secrets = Arc::new(MemorySecretStore::default());
        secrets
            .store(
                &papo_core::server_key(&url),
                Secret::SessionToken,
                SESSION_TOKEN,
            )
            .unwrap();
        let mut store = Store::default();
        store.selected_channel = CHANNEL_ID.into();
        let ticket = store.mark_loading(CHANNEL_ID);
        store.apply(papo_core::api::net::Update::Messages {
            ticket,
            messages: fixture_backend::fixture_messages(0..10_000).unwrap(),
            has_more: false,
            pinned_ids: None,
        });
        let runtime =
            ServerRuntime::open_with_store(url, store, wake, secrets, cache, notification);
        for _ in 0..DEFAULT_DRAIN_LIMIT {
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        runtime
    })
    .unwrap();
    let publications = owner.publications();
    assert_eq!(publications.capacity(), Some(1));
    let first = publications.recv_blocking().unwrap();
    assert_eq!(first.processed, DEFAULT_DRAIN_LIMIT);
    assert!(!first.update.upserts().is_empty());
    assert!(backend.stats().connections >= 1);
    assert!(backend.replay(2_000));
    assert!(backend.disconnect());
    // No foreground drain is needed to unblock the producer during teardown.
    owner.stop();
    owner.join().unwrap();
    assert!(backend.wait_for(Duration::from_secs(10), |stats| stats.active_sockets == 0));
}
