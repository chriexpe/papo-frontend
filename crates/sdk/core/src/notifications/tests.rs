//! Shared policy and FULL canonical identity checks without network access.

use super::*;
use crate::{
    Client, ClientConfig, DataRoot, UnixMicros,
    secret::{SecretKey, test_store::ScriptedSecretStore},
    session::SessionInner,
};
use std::sync::Arc;

pub(super) fn fixture(path: &std::path::Path) -> (Client, Session) {
    let client = Client::open(
        DataRoot::new(path),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let handle = client.handle();
    let server = crate::ServerAddress::parse("https://notification.invalid").unwrap();
    let account = AccountId::new(server.id(), Uuid::from_u128(1));
    let session = client
        .block_on(async {
            let registry = &handle.inner.registry;
            if registry.servers().await.unwrap().is_empty() {
                registry
                    .add_server(server.clone(), UnixMicros(0))
                    .await
                    .unwrap();
            }
            let saved = if let Some(saved) = registry
                .accounts(server.id())
                .await
                .unwrap()
                .into_iter()
                .next()
            {
                saved.credential_generation
            } else {
                registry
                    .add_account(account, None, None, true, UnixMicros(0))
                    .await
                    .unwrap()
            };
            let inner = SessionInner::new(
                account,
                server,
                SecretKey::new(account, saved),
                Arc::clone(&handle.inner.pool.next),
                handle.inner.pool.root.child_token(),
                crate::AuthState::Ready,
            );
            handle.inner.pool.insert(Arc::clone(&inner)).await;
            let generation = inner.epochs.current().generation;
            let mut resources = crate::ResourceSnapshot::empty(generation);
            resources.channels = vec![crate::Channel {
                id: Some(Uuid::from_u128(2).to_string()),
                notification_settings: Some("only_mentions".into()),
                ..Default::default()
            }]
            .into();
            inner.publish_resources(resources).unwrap();
            super::backend::remember(&inner, generation, None).unwrap();
            Session {
                inner,
                client: handle.clone(),
            }
        })
        .unwrap();
    (client, session)
}
pub(super) fn candidate() -> policy::Candidate {
    policy::Candidate {
        channel: Uuid::from_u128(2),
        message: Uuid::from_u128(3),
        author: Some(Uuid::from_u128(4)),
        content: String::new(),
        triggered: true,
        read: false,
        trigger_revision: None,
        observed_at_us: 0,
        message_point: None,
    }
}
#[test]
fn own_visible_muted_disabled_and_non_triggered_candidates_are_suppressed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    let generation = session.generation();
    let backend = papo_api::wire::Notifications::default();
    let mut settings = NotificationSettings::default();
    let mut item = candidate();
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .unwrap()
        .preview
        .is_none()
    );
    item.author = Some(session.account().user());
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    item = candidate();
    session
        .notifications()
        .set_visible_channels([item.channel].into())
        .unwrap();
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    session
        .notifications()
        .set_visible_channels(BTreeSet::default())
        .unwrap();
    settings.muted.insert(item.channel);
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    settings.muted.clear();
    settings.enabled = false;
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    settings.enabled = true;
    item.triggered = false;
    assert!(
        policy::decide(
            &session,
            generation,
            &settings,
            &backend,
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    client.shutdown().unwrap();
}
#[test]
fn canonical_full_claim_deduplicates_concurrent_producers_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let generation = session.generation();
            let message = Uuid::from_u128(3);
            let (a, b) = tokio::join!(
                ledger::claim(&session, generation, message, 0),
                ledger::claim(&session, generation, message, 0)
            );
            assert_ne!(a.unwrap(), b.unwrap());
            let connection = rusqlite::Connection::open(dir.path().join("state.sqlite")).unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM notification_ledger", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        })
        .unwrap();
    client.shutdown().unwrap();
    drop(session);
    drop(client);
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            assert!(
                !ledger::claim(&session, session.generation(), Uuid::from_u128(3), 1)
                    .await
                    .unwrap()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
#[test]
fn mute_changes_consume_suppressed_identity_and_stale_epochs_cannot_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let mut settings = NotificationSettings::default();
            settings.muted.insert(candidate().channel);
            session
                .notifications()
                .set_settings(settings)
                .await
                .unwrap();
            assert!(
                live::deliver_at(
                    &session,
                    session.generation(),
                    candidate(),
                    &Arc::clone(&session.inner.resources.borrow()),
                    0
                )
                .await
                .unwrap()
                .is_none()
            );
            session
                .notifications()
                .set_settings(NotificationSettings::default())
                .await
                .unwrap();
            assert!(
                live::deliver_at(
                    &session,
                    session.generation(),
                    candidate(),
                    &Arc::clone(&session.inner.resources.borrow()),
                    0
                )
                .await
                .unwrap()
                .is_none()
            );
            let old = session.generation();
            session.inner.epochs.advance().unwrap();
            assert!(
                ledger::claim(&session, old, Uuid::from_u128(9), 2)
                    .await
                    .is_err()
            );
            session.inner.root.cancel();
            assert!(
                ledger::claim(&session, session.generation(), Uuid::from_u128(10), 3)
                    .await
                    .is_err()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn pruned_identity_cannot_be_revived_by_an_old_gateway_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let resources = Arc::clone(&session.inner.resources.borrow());
            assert!(
                live::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                    .await
                    .unwrap()
                    .is_some()
            );
            let now_us = 31 * 24 * 60 * 60 * 1_000_000_i64;
            assert!(
                ledger::claim(&session, session.generation(), Uuid::from_u128(99), now_us)
                    .await
                    .unwrap()
            );
            assert!(
                live::deliver_at(
                    &session,
                    session.generation(),
                    candidate(),
                    &resources,
                    now_us
                )
                .await
                .unwrap()
                .is_none()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
#[test]
fn device_all_uses_shared_eligibility_and_own_message_never_alerts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let mut settings = NotificationSettings::default();
            settings
                .channels
                .insert(candidate().channel, ChannelNotificationMode::All);
            session
                .notifications()
                .set_settings(settings)
                .await
                .unwrap();
            let resources = Arc::clone(&session.inner.resources.borrow());
            let mut item = candidate();
            item.triggered = false;
            assert!(
                live::deliver_at(&session, session.generation(), item, &resources, 0)
                    .await
                    .unwrap()
                    .is_some()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn delayed_live_candidate_is_suppressed_after_a_newer_local_read_point() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    let mut image = (**session.inner.reading.image.borrow()).clone();
    image.channels.insert(
        candidate().channel,
        crate::ReadState {
            local_point: Some(crate::ReadPoint {
                created_at_us: 1,
                message_id: Uuid::from_u128(99),
            }),
            ..crate::ReadState::default()
        },
    );
    session.inner.reading.image.send_replace(Arc::new(image));
    let mut item = candidate();
    item.message_point = Some(crate::ReadPoint {
        created_at_us: 0,
        message_id: item.message,
    });
    assert!(
        policy::decide(
            &session,
            session.generation(),
            &NotificationSettings::default(),
            &papo_api::wire::Notifications::default(),
            &item,
            &session.inner.resources.borrow()
        )
        .is_none()
    );
    client.shutdown().unwrap();
}
