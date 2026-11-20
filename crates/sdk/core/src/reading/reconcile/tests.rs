//! A same-generation denial overtakes a reading rebuild without restoring its FULL state or fence.

use super::*;
use crate::history::access::{self, Refetch, Transition};
use crate::reading::positions::ChannelFence;
use papo_api::wire::{Channel, ChannelLastMessage, Server};

#[test]
fn a_completed_channel_denial_wins_over_an_older_reading_rebuild() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = crate::notifications::tests::fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let id = Uuid::from_u128(2);
            let healthy = Uuid::from_u128(3);
            let (connection, snapshot) = admit(&session, id, healthy).await;
            assert_eq!(
                session.inner.history.admission.read().await.channel(id),
                Some(ChannelAdmission::Allowed)
            );
            // This test drives refresh directly, without the periodic reading worker.
            session.inner.reading.started.store(true, Ordering::Release);
            let blocked = session.inner.reading.unresolved.lock().await;
            let mut rebuilding = std::pin::pin!(resources(&session.inner, &connection, &snapshot));
            tokio::select! {
                biased;
                result = &mut rebuilding => panic!("rebuild unexpectedly finished: {result:?}"),
                () = std::future::ready(()) => {},
            }
            assert!(session.inner.reading.serial.try_lock().is_err());
            delete(&session, &connection, id).await;
            drop(blocked);
            rebuilding.await.unwrap();

            assert_eq!(
                session
                    .inner
                    .reading
                    .positions
                    .check_current_channel(id, generation),
                Some(ChannelFence::Closed(
                    crate::HistoryClosedReason::ChannelDeleted
                ))
            );
            assert!(session.reading_state(id).is_none());
            assert!(
                session
                    .reading_state(healthy)
                    .unwrap()
                    .uploaded_point
                    .is_some()
            );
            let stored = session
                .client
                .inner
                .registry
                .load_read_points(session.account())
                .await
                .unwrap();
            assert!(!stored.contains_key(&id));
            assert!(stored[&healthy].uploaded.is_some());

            // A standalone denial must also remove a previously published row. A
            // refresh that started before the delete cannot restore its captured state.
            let stale = (**session.inner.reading.image.borrow()).clone();
            delete(&session, &connection, healthy).await;
            assert!(session.reading_state(healthy).is_none());
            session
                .inner
                .reading
                .publish(&session.inner, stale)
                .await
                .unwrap();
            assert!(session.reading_state(healthy).is_none());
        })
        .unwrap();
    client.shutdown().unwrap();
}

async fn admit(
    session: &Session,
    id: Uuid,
    healthy: Uuid,
) -> (ResourceConnection, ResourceSnapshot) {
    let generation = session.generation();
    let own = session.account().user();
    let channel = |id: Uuid| Channel {
        id: Some(id.to_string()),
        r#type: Some("text".into()),
        permissions: Some(vec![]),
        last_read_message: Some(id.to_string()),
        last_message: Some(ChannelLastMessage {
            id: Some(id.to_string()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let snapshot = ResourceSnapshot::fixture(
        generation,
        Some(Server {
            id: Some(Uuid::from_u128(10).to_string()),
            owner_id: Some(own.to_string()),
            ..Default::default()
        }),
        vec![channel(id), channel(healthy)],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        own,
    )
    .unwrap();
    let connection = ResourceConnection {
        client: session.client.clone(),
        cache: session
            .client
            .inner
            .caches
            .open(session.account())
            .await
            .unwrap(),
        rest: papo_api::RestClient::new(session.inner.address.as_str()).unwrap(),
    };
    let evidence = access::evidence(&session.inner, Refetch::Complete).await;
    let transition = Transition::begin(&session.inner, &connection, &snapshot, evidence)
        .await
        .unwrap();
    transition
        .finish(&session.inner, &connection)
        .await
        .unwrap();
    (connection, snapshot)
}

async fn delete(session: &Session, connection: &ResourceConnection, id: Uuid) {
    use papo_api::gateway::{ChannelDeleteEvent, Event, KnownEvent};
    let event = Event::Known(KnownEvent::ChannelDelete(Box::new(ChannelDeleteEvent {
        channel_id: id.to_string(),
    })));
    crate::history::live::apply(&session.inner, connection, session.generation(), &event)
        .await
        .unwrap();
}
