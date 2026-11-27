//! Shared policy and FULL canonical identity checks without network access.

use super::{
    deliver::{self, DeliveryOutcome},
    policy::PolicyResources,
    *,
};
use crate::{
    Client, ClientConfig, DataRoot, UnixMicros,
    reading::evidence::ReadEvidence,
    secret::{SecretKey, test_store::ScriptedSecretStore},
    session::SessionInner,
};
use std::sync::Arc;

pub(crate) fn fixture(path: &std::path::Path) -> (Client, Session) {
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
                    .add_account(account, "fixture".into(), None, None, true, UnixMicros(0))
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
            let resources = crate::ResourceSnapshot::fixture(
                generation,
                None,
                vec![crate::Channel {
                    r#type: Some("text".into()),
                    permissions: Some(vec![]),
                    id: Some(Uuid::from_u128(2).to_string()),
                    notification_settings: Some("only_mentions".into()),
                    ..Default::default()
                }],
                vec![],
                vec![],
                vec![],
                vec![],
                vec![],
                account.user(),
            )
            .unwrap();
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
        external_author: None,
        content: String::new(),
        attachments: Vec::new(),
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
    let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
    let decision = |settings: &NotificationSettings, item: &policy::Candidate| {
        policy::decide(
            &session,
            generation,
            settings,
            &backend,
            item,
            &resources,
            ReadEvidence::Unread,
        )
        .unwrap()
    };
    assert!(decision(&settings, &item).unwrap().preview.is_none());
    item.author = Some(session.account().user());
    assert!(decision(&settings, &item).is_none());
    item = candidate();
    session
        .notifications()
        .set_visible_channels([item.channel].into())
        .unwrap();
    assert!(decision(&settings, &item).is_none());
    session
        .notifications()
        .set_visible_channels(BTreeSet::default())
        .unwrap();
    settings.muted.insert(item.channel);
    assert!(decision(&settings, &item).is_none());
    settings.muted.clear();
    settings.enabled = false;
    assert!(decision(&settings, &item).is_none());
    settings.enabled = true;
    item.triggered = false;
    assert!(decision(&settings, &item).is_none());
    client.shutdown().unwrap();
}
#[test]
fn a_resumed_unread_message_waits_for_its_unambiguous_resource_image() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    let mut image = (**session.inner.resources.borrow()).clone();
    let item = candidate();
    image.dms.set(
        item.channel,
        crate::DirectRecord {
            id: item.channel,
            peer_id: Uuid::from_u128(4),
            created_at: String::new(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        },
    );
    let result = policy::decide(
        &session,
        session.generation(),
        &NotificationSettings::default(),
        &papo_api::wire::Notifications::default(),
        &item,
        &PolicyResources::from_snapshot(&image),
        ReadEvidence::Unread,
    );
    assert!(result.is_err());
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
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
            session
                .notifications()
                .set_settings(NotificationSettings::default())
                .await
                .unwrap();
            assert!(matches!(
                deliver::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
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
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(Some(_))
            ));
            let now_us = 31 * 24 * 60 * 60 * 1_000_000_i64;
            assert!(
                ledger::claim(&session, session.generation(), Uuid::from_u128(99), now_us)
                    .await
                    .unwrap()
            );
            assert!(matches!(
                deliver::deliver_at(
                    &session,
                    session.generation(),
                    candidate(),
                    &resources,
                    now_us
                )
                .await
                .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
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
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            let mut item = candidate();
            item.triggered = false;
            assert!(matches!(
                deliver::deliver_at(&session, session.generation(), item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(Some(_))
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn delayed_live_candidate_is_suppressed_after_a_newer_local_read_point() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let mut image = (**session.inner.reading.image.borrow()).clone();
            image.channels.insert(
                candidate().channel,
                crate::ReadState {
                    local_point: Some(crate::MessageKey {
                        created_at_us: 1,
                        message_id: Uuid::from_u128(99),
                    }),
                    ..crate::ReadState::default()
                },
            );
            session.inner.reading.image.send_replace(Arc::new(image));
            let mut item = candidate();
            item.message_point = Some(crate::MessageKey {
                created_at_us: 0,
                message_id: item.message,
            });
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, session.generation(), item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn submicrosecond_inbox_cursor_is_rejected_like_history() {
    let cursor = super::inbox::InboxCursor {
        created_at: "2026-01-01T00:00:00.123456789Z".into(),
        id: Uuid::from_u128(1).to_string(),
    };
    assert!(matches!(cursor.key(), Err(NotificationError::InvalidData)));
    assert!(crate::MessageKey::parse(&cursor.created_at, &cursor.id).is_err());
}

#[test]
fn policy_mentions_uses_context_free_canonical_token_matcher() {
    let target = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
    let other = Uuid::from_u128(2);
    assert!(policy::mentions(
        "look `@mention(<@00000000-0000-4000-8000-000000000001>)`",
        target
    ));
    assert!(policy::mentions(
        "```rust\n// @MENTION(<@00000000-0000-4000-8000-000000000001>)\n```",
        target
    ));
    assert!(!policy::mentions(
        "plain @mention(<@00000000-0000-4000-8000-000000000001>) without closing",
        other
    ));
}

#[test]
fn delivery_carries_author_dm_and_redacts_attachments_when_preview_off() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    let generation = session.generation();
    let author_id = Uuid::from_u128(4);
    let dm_channel = Uuid::from_u128(20);
    let mut snapshot = (**session.inner.resources.borrow()).clone();
    snapshot.users.set(
        author_id,
        crate::UserRecord {
            id: author_id,
            username: Some("lia".into()),
            nickname: Some("Lia M.".into()),
            roles: Some(Vec::new()),
            banned: Some(false),
            status_message: None,
            typing: None,
            created_at: None,
        },
    );
    snapshot.dms.set(
        dm_channel,
        crate::DirectRecord {
            id: dm_channel,
            peer_id: author_id,
            created_at: String::new(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        },
    );
    session.inner.publish_resources(snapshot).unwrap();
    let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
    let backend = papo_api::wire::Notifications::default();
    let attachment = papo_api::wire::Attachment {
        id: Some(Uuid::from_u128(50).to_string()),
        original_file_name: Some("photo.png".into()),
        mime_type: Some("image/png".into()),
        ..Default::default()
    };

    let mut item = candidate();
    item.content = "hello".into();
    item.attachments = vec![attachment.clone()];
    let mut settings = NotificationSettings::default();
    let private = policy::decide(
        &session,
        generation,
        &settings,
        &backend,
        &item,
        &resources,
        ReadEvidence::Unread,
    )
    .unwrap()
    .unwrap();
    assert_eq!(private.author, Some(author_id));
    assert_eq!(private.author_name.as_deref(), Some("Lia M."));
    assert!(!private.is_dm);
    assert!(private.preview.is_none());
    assert!(private.attachments.is_empty());

    settings.preview = true;
    let shown = policy::decide(
        &session,
        generation,
        &settings,
        &backend,
        &item,
        &resources,
        ReadEvidence::Unread,
    )
    .unwrap()
    .unwrap();
    assert_eq!(shown.preview.as_deref(), Some("hello"));
    assert_eq!(shown.attachments.len(), 1);
    assert_eq!(
        shown.attachments[0].original_file_name.as_deref(),
        Some("photo.png")
    );

    item.channel = dm_channel;
    item.external_author = Some("Webhook Bot".into());
    let dm = policy::decide(
        &session,
        generation,
        &settings,
        &backend,
        &item,
        &resources,
        ReadEvidence::Unread,
    )
    .unwrap()
    .unwrap();
    assert!(dm.is_dm);
    assert_eq!(dm.author_name.as_deref(), Some("Webhook Bot"));
    client.shutdown().unwrap();
}

#[test]
fn account_policy_publishes_backend_notification_flags() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    let generation = session.generation();
    let mut rx = session.notifications().subscribe_account_policy();
    let initial = session.notifications().account_policy().unwrap();
    assert_eq!(
        initial,
        AccountNotificationPolicy {
            generation,
            enabled: true,
            mentions: true,
            message_preview: true,
            sound: true,
        }
    );
    super::backend::remember(
        &session.inner,
        generation,
        Some(papo_api::wire::WhoamiResponseSettings {
            version: Some(2),
            config: Some(papo_api::wire::UserConfig {
                notifications: Some(papo_api::wire::Notifications {
                    enabled: Some(true),
                    mentions: Some(false),
                    message_preview: Some(false),
                    sound: Some(false),
                }),
                ..Default::default()
            }),
        }),
    )
    .unwrap();
    assert!(rx.has_changed().unwrap());
    let updated = rx.borrow_and_update().unwrap();
    assert_eq!(
        updated,
        AccountNotificationPolicy {
            generation,
            enabled: true,
            mentions: false,
            message_preview: false,
            sound: false,
        }
    );
    client.shutdown().unwrap();
}
