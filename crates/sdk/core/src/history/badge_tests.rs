//! Cached badge integration: durable eligibility changes never fetch or deliver history.

use crate::{
    ChannelNotificationMode, HistoryChunk, HistorySnapshot, Message, MessageKey,
    NotificationSettings, ReadState, ReadingSnapshot, Session, UnreadCount, Uuid,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

fn snapshot(session: &Session) -> HistorySnapshot {
    let id = Uuid::from_u128(10).to_string();
    let created_at = "2026-01-01T00:00:00.000001Z".to_owned();
    let cursor = papo_api::HistoryCursor {
        id: id.clone(),
        created_at: created_at.clone(),
    };
    HistorySnapshot {
        version: 1,
        generation: session.generation(),
        messages: [Arc::new(Message {
            id: Some(id),
            created_at: Some(created_at),
            author_id: Some(Uuid::from_u128(4).to_string()),
            content: Some(crate::mentions::encode_user_mention(
                session.account().user(),
            )),
            ..Message::default()
        })]
        .into_iter()
        .collect(),
        chunks: vec![HistoryChunk {
            first: cursor.clone(),
            last: cursor,
            oldest_complete: true,
            newest_complete: true,
            generation: Some(session.generation()),
            live_generation: Some(session.generation()),
            oldest_anchor: None,
            newest_anchor: None,
        }],
        tombstones: imbl::HashSet::new(),
        gaps: Vec::new(),
        empty_complete: false,
        server_live_complete: true,
        live_arrival_start: None,
    }
}

async fn install(session: &Session, snapshot: HistorySnapshot) {
    let channel = Uuid::from_u128(2);
    let held = session.inner.history.channel(channel).await;
    held.publish(&*held.state.lock().await, session.generation())
        .unwrap();
    held.publication.lock().unwrap().snapshot = Some(Arc::new(snapshot));
    session
        .inner
        .reading
        .image
        .send_replace(Arc::new(ReadingSnapshot {
            generation: session.generation(),
            version: 0,
            channels: BTreeMap::from([(channel, ReadState::default())]),
        }));
    crate::reading::reconcile::refresh(session).await.unwrap();
}

#[test]
fn badges_recount_committed_settings_without_fetch_or_delivery_and_skip_unchanged_publications() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            session
                .notifications()
                .set_visible_channels(BTreeSet::from([channel]))
                .unwrap();
            install(&session, snapshot(&session)).await;
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(1)
            );
            let mut changes = session.subscribe_reading();
            let version = changes.borrow_and_update().version;
            let mut settings = NotificationSettings::default();
            settings.muted.insert(channel);
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert!(
                session
                    .notifications()
                    .settings()
                    .await
                    .unwrap()
                    .muted
                    .contains(&channel)
            );
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            assert_eq!(changes.borrow_and_update().version, version + 1);
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert!(!changes.has_changed().unwrap());
            settings.muted.clear();
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(1)
            );
            settings
                .channels
                .insert(channel, ChannelNotificationMode::Off);
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            settings
                .channels
                .insert(channel, ChannelNotificationMode::Mentions);
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            settings.mentions = false;
            session
                .notifications()
                .set_settings(settings)
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            assert!(
                session
                    .inner
                    .notifications
                    .receiver
                    .lock()
                    .await
                    .try_recv()
                    .is_err()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn gaps_lower_bound_read_clears_and_backend_modes_recount_using_incoming_resources() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let mut history = snapshot(&session);
            history.chunks[0].oldest_complete = false;
            install(&session, history.clone()).await;
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::AtLeast(1)
            );
            let mut resources = (**session.inner.resources.borrow()).clone();
            Arc::make_mut(&mut resources.channels)[0].notification_settings = Some("off".into());
            crate::reading::reconcile::refresh_with_resources(&session, &resources, None)
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            Arc::make_mut(&mut resources.channels)[0].notification_settings =
                Some("only_mentions".into());
            crate::reading::reconcile::refresh_with_resources(&session, &resources, None)
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::AtLeast(1)
            );
            history.chunks[0].oldest_complete = true;
            let point = MessageKey::from_message(&history.messages[0]).unwrap();
            install(&session, history).await;
            let mut reading = (**session.inner.reading.image.borrow()).clone();
            reading.channels.get_mut(&channel).unwrap().local_point = Some(point);
            session.inner.reading.image.send_replace(Arc::new(reading));
            crate::reading::reconcile::refresh(&session).await.unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn backend_disabled_mentions_still_recounts_device_all_mode_transition() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            session
                .inner
                .notifications
                .backend
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .config
                .mentions = Some(false);
            install(&session, snapshot(&session)).await;
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            let mut settings = NotificationSettings::default();
            settings
                .channels
                .insert(channel, ChannelNotificationMode::All);
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(1)
            );
            settings
                .channels
                .insert(channel, ChannelNotificationMode::Mentions);
            session
                .notifications()
                .set_settings(settings)
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn enabling_without_history_restores_unknown_and_dm_master_switches_recount() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            session
                .inner
                .reading
                .image
                .send_replace(Arc::new(ReadingSnapshot {
                    generation: session.generation(),
                    version: 0,
                    channels: BTreeMap::from([(channel, ReadState::default())]),
                }));
            let mut settings = NotificationSettings {
                enabled: false,
                ..Default::default()
            };
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            settings.enabled = true;
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::AtLeast(0)
            );
            let mut resources = (**session.inner.resources.borrow()).clone();
            resources.dms = vec![papo_api::wire::DirectConversation {
                id: channel.to_string(),
                ..Default::default()
            }]
            .into();
            session.inner.publish_resources(resources).unwrap();
            install(&session, snapshot(&session)).await;
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(1)
            );
            settings.direct_messages = false;
            session
                .notifications()
                .set_settings(settings.clone())
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(0)
            );
            settings.direct_messages = true;
            session
                .notifications()
                .set_settings(settings)
                .await
                .unwrap();
            assert_eq!(
                session.reading_state(channel).unwrap().mention_count,
                UnreadCount::Exact(1)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
