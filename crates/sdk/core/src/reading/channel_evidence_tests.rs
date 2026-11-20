//! Batched channel read-boundary evidence and dependency keys over real SQLite; notification
//! candidate compatibility stays covered by the original evidence tests.

use super::*;
use crate::{
    notifications::tests::fixture,
    read_points::{ServerReadPoint, UploadedPoint},
};
use std::sync::Arc;

fn key(created_at_us: i64, id: u128) -> MessageKey {
    MessageKey {
        created_at_us,
        message_id: Uuid::from_u128(id),
    }
}

#[test]
fn channel_evidence_requires_current_published_boundary_even_without_a_stored_row() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            let candidate = key(100, 11);
            let cold = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(cold.ready);
            assert!(!cold.boundary_known());
            assert_eq!(cold.point(), None);
            assert_eq!(cold.head(), None);
            session
                .inner
                .reading
                .heads
                .lock()
                .await
                .insert(channel, candidate);
            assert_eq!(
                cold.message(candidate.message_id, Some(candidate)),
                ReadEvidence::Unread
            );
            assert_eq!(
                cold.badge_message(candidate.message_id, Some(candidate)),
                ReadEvidence::NotReady
            );

            let mut image = (**session.inner.reading.image.borrow()).clone();
            image.channels.insert(channel, crate::ReadState::default());
            session
                .inner
                .reading
                .publish(&session.inner, image)
                .await
                .unwrap();
            let known_empty = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(known_empty.boundary_known());
            assert_eq!(known_empty.head(), Some(candidate));
            assert_eq!(
                known_empty.badge_message(candidate.message_id, Some(candidate)),
                ReadEvidence::Unread
            );
            let resolved = known_empty.with_point(Some(key(100, 12)));
            assert_eq!(
                resolved.badge_message(candidate.message_id, Some(candidate)),
                ReadEvidence::Read
            );
            assert_eq!(resolved.point(), Some(key(100, 12)));

            let mut image = (**session.inner.reading.image.borrow()).clone();
            image.generation = crate::Generation(generation.get() + 1);
            image.channels.get_mut(&channel).unwrap().local_point = Some(key(200, 20));
            session.inner.reading.image.send_replace(Arc::new(image));
            let stale = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(!stale.boundary_known());
            assert_eq!(stale.head(), None);
            assert_eq!(
                stale.badge_message(candidate.message_id, Some(candidate)),
                ReadEvidence::NotReady
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn boundary_inputs_change_without_public_read_state_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            let mut image = (**session.inner.reading.image.borrow()).clone();
            image.channels.insert(channel, crate::ReadState::default());
            session
                .inner
                .reading
                .publish(&session.inner, image)
                .await
                .unwrap();
            let public_state = session.reading_state(channel);
            let initial = channel_boundaries(&session.inner).await;
            assert!(!initial.contains_key(&channel));
            assert!(
                channel_read_evidence(&session, generation, channel)
                    .await
                    .unwrap()
                    .boundary_known()
            );

            let head = key(100, 10);
            session
                .inner
                .reading
                .heads
                .lock()
                .await
                .insert(channel, head);
            let headed = channel_boundaries(&session.inner).await;
            assert_ne!(headed, initial);
            assert_eq!(
                headed.get(&channel),
                Some(&ChannelBoundary {
                    head: Some(head),
                    unresolved: None
                })
            );
            assert_eq!(session.reading_state(channel), public_state);

            let unresolved = Uuid::from_u128(500);
            session.inner.reading.unresolved.lock().await.insert(
                channel,
                ServerReadPoint {
                    message_id: unresolved,
                    recorded_at: None,
                },
            );
            let marked = channel_boundaries(&session.inner).await;
            assert_ne!(marked, headed);
            assert_eq!(
                marked.get(&channel),
                Some(&ChannelBoundary {
                    head: Some(head),
                    unresolved: Some(unresolved)
                })
            );
            assert_eq!(session.reading_state(channel), public_state);
            let evidence = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(!evidence.boundary_known());
            assert_eq!(
                evidence.badge_message(head.message_id, Some(head)),
                ReadEvidence::NotReady
            );

            session.inner.reading.heads.lock().await.remove(&channel);
            let mark_only = channel_boundaries(&session.inner).await;
            assert_eq!(
                mark_only.get(&channel),
                Some(&ChannelBoundary {
                    head: None,
                    unresolved: Some(unresolved)
                })
            );
            assert_eq!(session.reading_state(channel), public_state);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn resolved_legacy_point_replaces_imprecise_order_before_pure_evaluation() {
    let evidence = ChannelReadEvidence {
        ready: true,
        boundary_known: true,
        point: Some(key(100, 10)),
        ..ChannelReadEvidence::default()
    }
    .with_point(Some(key(99, 10)));

    assert_eq!(evidence.point(), Some(key(99, 10)));
    assert_eq!(
        evidence.badge_message(Uuid::from_u128(9), Some(key(100, 9))),
        ReadEvidence::Unread
    );
}

#[test]
fn unresolved_server_id_preserves_known_maximum_but_never_proves_badge_unread() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            let known = key(100, 10);
            let server = ServerReadPoint {
                message_id: Uuid::from_u128(500),
                recorded_at: None,
            };
            session
                .client
                .inner
                .registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server: server.clone(),
                        created_at_us: None,
                        known_point: Some(known),
                    },
                )
                .await
                .unwrap();
            let mut image = (**session.inner.reading.image.borrow()).clone();
            image.channels.insert(channel, crate::ReadState::default());
            session
                .inner
                .reading
                .publish(&session.inner, image)
                .await
                .unwrap();
            let durable_unresolved = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert_eq!(durable_unresolved.point(), Some(known));
            assert!(!durable_unresolved.boundary_known());
            assert_eq!(
                durable_unresolved.badge_message(server.message_id, None),
                ReadEvidence::Read
            );
            assert_eq!(
                durable_unresolved.badge_message(Uuid::from_u128(9), Some(key(100, 9))),
                ReadEvidence::Read
            );
            assert_eq!(
                durable_unresolved.badge_message(Uuid::from_u128(11), Some(key(100, 11))),
                ReadEvidence::NotReady
            );
            assert_eq!(
                durable_unresolved.message(Uuid::from_u128(11), Some(key(100, 11))),
                ReadEvidence::Unread
            );

            session
                .client
                .inner
                .registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server,
                        created_at_us: Some(50),
                        known_point: Some(known),
                    },
                )
                .await
                .unwrap();
            let resolved = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(resolved.boundary_known());
            assert_eq!(resolved.point(), Some(known));
            session.inner.reading.unresolved.lock().await.insert(
                channel,
                ServerReadPoint {
                    message_id: Uuid::from_u128(600),
                    recorded_at: None,
                },
            );
            let live_unresolved = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(!live_unresolved.boundary_known());
            assert_eq!(
                live_unresolved.badge_message(Uuid::from_u128(600), None),
                ReadEvidence::Read
            );
            assert_eq!(
                live_unresolved.badge_message(Uuid::from_u128(11), Some(key(100, 11))),
                ReadEvidence::NotReady
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
