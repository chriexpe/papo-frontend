//! Commit-safe notification policy invalidation; no optimistic settings image or network is involved.

use super::{NotificationSettings, backend, ledger, tests::fixture};
use crate::StorageError;
use papo_api::wire::{Notifications, UserConfig, WhoamiResponseSettings};
use std::{future::Future, sync::mpsc, task::Poll};

#[test]
fn settings_signal_waits_for_full_commit_and_observers_load_the_committed_image() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let changes = session.inner.notifications.policy_changes.subscribe();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let registry = session.client.inner.registry.clone();
            let holder = tokio::spawn(async move {
                registry
                    .writer
                    .write(move |_| {
                        entered_tx.send(()).unwrap();
                        let _ = release_rx.recv();
                        Ok::<_, StorageError>(())
                    })
                    .await
            });
            tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
                .await
                .unwrap();
            let settings = NotificationSettings {
                enabled: false,
                ..Default::default()
            };
            let save = ledger::save_settings(&session, settings.clone());
            tokio::pin!(save);
            std::future::poll_fn(|cx| {
                assert!(save.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            assert!(!changes.has_changed().unwrap());
            assert!(ledger::settings(&session).await.unwrap().enabled);
            release_tx.send(()).unwrap();
            holder.await.unwrap().unwrap();
            save.await.unwrap();
            assert!(changes.has_changed().unwrap());
            assert!(ledger::settings(&session).await.unwrap() == settings);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn failed_settings_save_does_not_signal_or_publish_optimistic_policy() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    let connection = rusqlite::Connection::open(directory.path().join("state.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_notification_settings BEFORE INSERT ON notification_state
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END",
        )
        .unwrap();
    client
        .block_on(async {
            let changes = session.inner.notifications.policy_changes.subscribe();
            let settings = NotificationSettings {
                enabled: false,
                ..Default::default()
            };
            assert!(ledger::save_settings(&session, settings).await.is_err());
            assert!(!changes.has_changed().unwrap());
            assert!(ledger::settings(&session).await.unwrap().enabled);
        })
        .unwrap();
    client.shutdown().unwrap();
}

fn backend_settings(version: i64, notifications: Notifications) -> WhoamiResponseSettings {
    WhoamiResponseSettings {
        version: Some(version),
        config: Some(UserConfig {
            notifications: Some(notifications),
            ..Default::default()
        }),
    }
}

#[test]
fn backend_signal_requires_accepted_effective_change_not_stale_or_default_equivalent_fields() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    let generation = session.generation();
    let mut changes = session.inner.notifications.policy_changes.subscribe();
    let explicit_defaults = Notifications {
        enabled: Some(true),
        mentions: Some(true),
        message_preview: Some(true),
        sound: Some(true),
    };
    backend::remember(
        &session.inner,
        generation,
        Some(backend_settings(1, explicit_defaults)),
    )
    .unwrap();
    assert!(!changes.has_changed().unwrap());
    let disabled = Notifications {
        enabled: Some(false),
        ..Default::default()
    };
    backend::remember(
        &session.inner,
        generation,
        Some(backend_settings(2, disabled.clone())),
    )
    .unwrap();
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    backend::remember(
        &session.inner,
        generation,
        Some(backend_settings(3, disabled)),
    )
    .unwrap();
    assert!(!changes.has_changed().unwrap());
    backend::remember(
        &session.inner,
        generation,
        Some(backend_settings(2, Notifications::default())),
    )
    .unwrap();
    assert!(!changes.has_changed().unwrap());
    assert_eq!(
        session
            .inner
            .notifications
            .backend
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .config
            .enabled,
        Some(false)
    );
    backend::remember(
        &session.inner,
        generation,
        Some(backend_settings(4, Notifications::default())),
    )
    .unwrap();
    assert!(changes.has_changed().unwrap());
    client.shutdown().unwrap();
}
