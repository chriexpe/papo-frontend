//! Badge coverage/read/classification regressions with reading-owned SQLite evidence.

#[test]
fn unchanged_badge_images_never_republish_or_replace_their_arcs() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    let channel = Uuid::from_u128(2);
    let badge = Arc::new(ChannelBadges {
        has_unread: Some(true),
        mention_count: UnreadCount::AtLeast(1),
        direct_message: false,
    });
    let mut updates = session.inner.badges.image.subscribe();
    recount::publish(
        &session.inner,
        session.generation(),
        true,
        [(channel, Arc::clone(&badge))].into(),
    );
    let first = updates.borrow_and_update().clone();
    recount::publish(
        &session.inner,
        session.generation(),
        true,
        [(channel, Arc::clone(&badge))].into(),
    );
    assert!(!updates.has_changed().unwrap());
    assert!(Arc::ptr_eq(&first, &updates.borrow()));
    assert!(Arc::ptr_eq(&badge, &updates.borrow().channels[&channel]));
    client.shutdown().unwrap();
}

use super::*;
use crate::{
    HistoryChunk, HistorySnapshot, Message, MessageKey, NotificationSettings, ReadState,
    ReadingSnapshot, notifications::policy::ChannelPolicy,
    reading::evidence::channel_read_evidence,
};

fn message(index: u128, user: Uuid) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(index).to_string()),
        author_id: Some(Uuid::from_u128(4).to_string()),
        channel_id: Some(Uuid::from_u128(2).to_string()),
        created_at: Some(format!("2026-01-01T00:00:{index:02}.000000Z")),
        content: Some(crate::encode_user_mention(user)),
        ..Message::default()
    })
}
fn snapshot(
    generation: Generation,
    messages: Vec<Arc<Message>>,
    complete: bool,
) -> HistorySnapshot {
    let first = messages
        .first()
        .map(|m| MessageKey::from_message(m).unwrap().to_cursor().unwrap());
    let last = messages
        .last()
        .map(|m| MessageKey::from_message(m).unwrap().to_cursor().unwrap());
    HistorySnapshot {
        version: 1,
        generation,
        messages: messages.into_iter().collect(),
        tombstones: imbl::HashSet::default(),
        chunks: first
            .zip(last)
            .map(|(first, last)| HistoryChunk {
                first,
                last,
                oldest_complete: complete,
                newest_complete: complete,
                generation: Some(generation),
                live_generation: Some(generation),
                oldest_anchor: None,
                newest_anchor: None,
            })
            .into_iter()
            .collect(),
        gaps: Vec::new(),
        empty_complete: false,
        server_live_complete: complete,
        live_arrival_start: None,
    }
}
fn policy(settings: &NotificationSettings) -> ChannelPolicy {
    ChannelPolicy::new(
        settings,
        &papo_api::wire::Notifications::default(),
        Uuid::from_u128(2),
        false,
        Some(&crate::BackendMode::All),
    )
}
async fn install_reading(session: &Session, state: ReadState) {
    session
        .inner
        .reading
        .publish(
            &session.inner,
            ReadingSnapshot {
                generation: session.generation(),
                version: 0,
                channels: [(Uuid::from_u128(2), state)].into(),
            },
        )
        .await
        .unwrap();
}

#[test]
fn missing_boundary_never_proves_exact_even_with_complete_history() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let snapshot = snapshot(
                session.generation(),
                vec![message(1, session.account().user())],
                true,
            );
            let evidence =
                channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                    .await
                    .unwrap();
            assert_eq!(
                counts::mentions(
                    &snapshot,
                    evidence,
                    policy(&NotificationSettings::default()),
                    session.account().user(),
                    false
                )
                .unwrap(),
                UnreadCount::AtLeast(0)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn complete_and_incomplete_ranges_and_material_unknowns_remain_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            install_reading(&session, ReadState::default()).await;
            let mut direct = (*message(1, session.account().user())).clone();
            direct.content = Some(format!(
                "{} {}",
                "x".repeat(600),
                crate::encode_user_mention(session.account().user())
            ));
            direct.reply_to = Some(Uuid::from_u128(99).to_string());
            for (complete, expected) in [
                (true, UnreadCount::Exact(1)),
                (false, UnreadCount::AtLeast(1)),
            ] {
                let snapshot = snapshot(
                    session.generation(),
                    vec![Arc::new(direct.clone())],
                    complete,
                );
                let evidence =
                    channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                        .await
                        .unwrap();
                assert_eq!(
                    counts::mentions(
                        &snapshot,
                        evidence,
                        policy(&NotificationSettings::default()),
                        session.account().user(),
                        false
                    )
                    .unwrap(),
                    expected
                );
            }
            let mut reply = (*message(2, session.account().user())).clone();
            reply.content = Some("ordinary".into());
            reply.reply_to = Some(Uuid::from_u128(99).to_string());
            let snapshot = snapshot(
                session.generation(),
                vec![Arc::new(direct), Arc::new(reply)],
                true,
            );
            let evidence =
                channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                    .await
                    .unwrap();
            assert_eq!(
                counts::mentions(
                    &snapshot,
                    evidence,
                    policy(&NotificationSettings::default()),
                    session.account().user(),
                    false
                )
                .unwrap(),
                UnreadCount::AtLeast(1)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn parent_arrival_read_movement_and_policy_filtering_use_their_owners() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            install_reading(&session, ReadState::default()).await;
            let mut parent = (*message(1, session.account().user())).clone();
            parent.author_id = Some(session.account().user().to_string());
            let mut reply = (*message(2, session.account().user())).clone();
            reply.content = Some("ordinary".into());
            reply.reply_to = parent.id.clone();
            let reply = Arc::new(reply);
            for (messages, expected) in [
                (vec![Arc::clone(&reply)], UnreadCount::AtLeast(0)),
                (
                    vec![Arc::new(parent), Arc::clone(&reply)],
                    UnreadCount::Exact(1),
                ),
            ] {
                let snapshot = snapshot(session.generation(), messages, true);
                let evidence =
                    channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                        .await
                        .unwrap();
                assert_eq!(
                    counts::mentions(
                        &snapshot,
                        evidence,
                        policy(&NotificationSettings::default()),
                        session.account().user(),
                        false
                    )
                    .unwrap(),
                    expected
                );
            }
            install_reading(
                &session,
                ReadState {
                    local_point: Some(MessageKey::from_message(&reply).unwrap()),
                    ..ReadState::default()
                },
            )
            .await;
            let snapshot = snapshot(session.generation(), vec![reply], true);
            let evidence =
                channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                    .await
                    .unwrap();
            assert_eq!(
                counts::mentions(
                    &snapshot,
                    evidence,
                    policy(&NotificationSettings::default()),
                    session.account().user(),
                    false
                )
                .unwrap(),
                UnreadCount::Exact(0)
            );
            let settings = NotificationSettings {
                enabled: false,
                ..NotificationSettings::default()
            };
            let evidence =
                channel_read_evidence(&session, session.generation(), Uuid::from_u128(2))
                    .await
                    .unwrap();
            assert_eq!(
                counts::mentions(
                    &snapshot,
                    evidence,
                    policy(&settings),
                    session.account().user(),
                    false
                )
                .unwrap(),
                UnreadCount::Exact(0)
            );
            assert_eq!(counts::has_unread(UnreadCount::AtLeast(1)), Some(true));
            assert_eq!(counts::has_unread(UnreadCount::AtLeast(0)), None);
        })
        .unwrap();
    client.shutdown().unwrap();
}
