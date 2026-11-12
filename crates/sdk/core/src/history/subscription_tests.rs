//! Deterministic raw subscription ordering, coalescing and lifecycle contracts.

use std::sync::Arc;

use papo_api::{HistoryOrder, Uuid, wire::Message};

use crate::Generation;

use super::{
    HistoryClosedReason, HistoryError, HistoryUpdate, channel::Channel, merge::Ticket,
    persistence::State,
};

const AT: &str = "2026-01-01T00:00:00.123456Z";

fn message(id: u128) -> Message {
    Message {
        id: Some(Uuid::from_u128(id).to_string()),
        channel_id: Some(Uuid::nil().to_string()),
        created_at: Some(AT.into()),
        ..Message::default()
    }
}

fn merge(state: &mut State, messages: &[Message]) {
    let ticket = Ticket {
        channel: *Uuid::nil().as_bytes(),
        generation: 1,
        revision: state.model.revision,
        cursor: None,
        order: HistoryOrder::Desc,
        limit: 100,
    };
    state.model.merge_page(&ticket, messages, false).unwrap();
}

fn channel() -> Channel {
    Channel::new(
        Uuid::nil(),
        tokio::sync::watch::channel(()).0,
        Arc::default(),
    )
}

#[tokio::test]
async fn atomic_subscription_starts_seen_and_gets_the_next_complete_snapshot() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    merge(&mut state, &[message(2), message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let mut subscription = channel.subscribe().unwrap();
    assert!(!subscription.updates.has_changed().unwrap());
    assert_eq!(subscription.snapshot.messages[0].id, message(1).id);
    assert_eq!(subscription.snapshot.messages[1].id, message(2).id);
    merge(&mut state, &[message(3), message(2), message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    subscription.updates.changed().await.unwrap();
    let HistoryUpdate::Reset(snapshot) = &*subscription.updates.borrow_and_update() else {
        panic!("expected raw reset");
    };
    assert_eq!(snapshot.version, subscription.snapshot.version + 1);
    assert_eq!(snapshot.messages.len(), 3);
    assert_eq!(snapshot.chunks[0].first.created_at, AT);
}

#[tokio::test]
async fn slow_subscriber_coalesces_to_latest_coherent_reset() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    channel.publish(&state, Generation(1)).unwrap();
    let mut subscription = channel.subscribe().unwrap();
    for id in 1..=100 {
        let messages: Vec<_> = (1..=id).rev().map(message).collect();
        merge(&mut state, &messages);
        channel.publish(&state, Generation(1)).unwrap();
    }
    subscription.updates.changed().await.unwrap();
    let HistoryUpdate::Reset(snapshot) = &*subscription.updates.borrow_and_update() else {
        panic!("expected raw reset");
    };
    assert_eq!(snapshot.version, subscription.snapshot.version + 100);
    assert_eq!(snapshot.messages.len(), 100);
    assert_eq!(snapshot.messages.last().unwrap().id, message(100).id);
}

#[tokio::test]
async fn generation_only_publication_preserves_unchanged_message_identity() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    merge(&mut state, &[message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let before = channel.subscribe().unwrap().snapshot;
    channel.publish(&state, Generation(2)).unwrap();
    let after = channel.subscribe().unwrap().snapshot;
    assert_eq!(after.generation, Generation(2));
    assert_eq!(after.version, before.version + 1);
    assert!(Arc::ptr_eq(&before.messages[0], &after.messages[0]));
    assert_eq!(after.messages[0].cursor().unwrap().created_at, AT);
}

#[tokio::test]
async fn closed_reason_is_sticky_and_cannot_be_overwritten_by_reset() {
    let channel = channel();
    let state = channel.state.lock().await;
    channel.publish(&state, Generation(1)).unwrap();
    let mut subscription = channel.subscribe().unwrap();
    channel.close(HistoryClosedReason::Forbidden);
    channel.close(HistoryClosedReason::AccountClosed);
    assert!(matches!(
        channel.publish(&state, Generation(2)),
        Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
    ));
    subscription.updates.changed().await.unwrap();
    assert!(matches!(
        *subscription.updates.borrow_and_update(),
        HistoryUpdate::Closed(HistoryClosedReason::Forbidden)
    ));
    assert!(matches!(
        channel.subscribe(),
        Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
    ));
}

#[tokio::test]
async fn detached_pins_never_enter_chronological_snapshot() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    let ticket = Ticket {
        channel: *Uuid::nil().as_bytes(),
        generation: 1,
        revision: 0,
        cursor: None,
        order: HistoryOrder::Desc,
        limit: 100,
    };
    state.model.pins(&ticket, &[message(1)]).unwrap();
    channel.publish(&state, Generation(1)).unwrap();
    let snapshot = channel.subscribe().unwrap().snapshot;
    assert!(snapshot.messages.is_empty());
    assert!(snapshot.chunks.is_empty());
}

#[tokio::test]
async fn an_edit_replaces_only_the_changed_payload_arc() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    merge(&mut state, &[message(2), message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let before = channel.subscribe().unwrap().snapshot;
    state
        .model
        .event(
            &papo_api::gateway::KnownEvent::MessageEdit(Box::new(
                papo_api::gateway::MessageEditEvent {
                    id: message(1).id.unwrap(),
                    channel_id: Uuid::nil().to_string(),
                    content: String::new(),
                    edited_at: "2026-01-01T00:00:01.123456Z".into(),
                },
            )),
            1,
        )
        .unwrap();
    channel.publish(&state, Generation(1)).unwrap();
    let after = channel.subscribe().unwrap().snapshot;
    assert!(!Arc::ptr_eq(&before.messages[0], &after.messages[0]));
    assert!(Arc::ptr_eq(&before.messages[1], &after.messages[1]));
}

#[tokio::test]
async fn equal_payload_refetch_keeps_the_original_arc_despite_model_revision() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    merge(&mut state, &[message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let before = channel.subscribe().unwrap().snapshot;
    merge(&mut state, &[message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let after = channel.subscribe().unwrap().snapshot;
    assert!(Arc::ptr_eq(&before.messages[0], &after.messages[0]));
    assert_eq!(after.version, before.version + 1);
}

#[tokio::test]
async fn epoch_restamp_uses_last_accepted_snapshot_not_an_unpublished_mutation() {
    let store = super::channel::HistoryStore::default();
    let channel = Arc::new(Channel::new(
        Uuid::nil(),
        store.changes.clone(),
        Arc::clone(&store.counters),
    ));
    store
        .channels
        .lock()
        .await
        .insert(Uuid::nil(), Arc::clone(&channel));
    let mut state = channel.state.lock().await;
    merge(&mut state, &[message(1)]);
    channel.publish(&state, Generation(1)).unwrap();
    let before = channel.subscribe().unwrap().snapshot;
    merge(&mut state, &[message(2), message(1)]);
    store.generation_changed(Generation(2)).await.unwrap();
    let after = channel.subscribe().unwrap().snapshot;
    assert_eq!(after.messages.len(), 1);
    assert_eq!(after.generation, Generation(2));
    assert!(Arc::ptr_eq(&before.messages[0], &after.messages[0]));
    assert_eq!(after.chunks[0].live_generation, Some(Generation(1)));
}

#[tokio::test]
async fn idle_account_close_wakes_subscriber_without_waiting_for_the_model_lock() {
    let store = super::channel::HistoryStore::default();
    let channel = Arc::new(Channel::new(
        Uuid::nil(),
        store.changes.clone(),
        Arc::clone(&store.counters),
    ));
    store
        .channels
        .lock()
        .await
        .insert(Uuid::nil(), Arc::clone(&channel));
    let state = channel.state.lock().await;
    channel.publish(&state, Generation(1)).unwrap();
    let mut subscription = channel.subscribe().unwrap();
    store.close_all().await;
    subscription.updates.changed().await.unwrap();
    assert!(matches!(
        *subscription.updates.borrow_and_update(),
        HistoryUpdate::Closed(HistoryClosedReason::AccountClosed)
    ));
    assert!(store.snapshots().await.is_empty());
}

#[tokio::test]
async fn a_watch_receiver_keeps_history_active_until_it_is_dropped() {
    let channel = channel();
    let state = channel.state.lock().await;
    channel.publish(&state, Generation(1)).unwrap();
    assert!(!channel.is_open());
    let subscription = channel.subscribe().unwrap();
    assert!(channel.is_open());
    drop(subscription);
    assert!(!channel.is_open());
}

#[tokio::test]
async fn raw_coverage_preserves_unknown_gap_boundaries_and_exact_cursors() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    let head = Ticket {
        channel: *Uuid::nil().as_bytes(),
        generation: 1,
        revision: 0,
        cursor: None,
        order: HistoryOrder::Desc,
        limit: 2,
    };
    state
        .model
        .merge_page(&head, &[message(9), message(8)], true)
        .unwrap();
    let older = Ticket {
        revision: state.model.revision,
        cursor: Some(super::ordering::Point::from(&message(4).cursor().unwrap())),
        ..head
    };
    state
        .model
        .merge_page(&older, &[message(3), message(2)], false)
        .unwrap();
    channel.publish(&state, Generation(1)).unwrap();
    let snapshot = channel.subscribe().unwrap().snapshot;
    assert_eq!(snapshot.chunks.len(), 2);
    assert_eq!(snapshot.messages.len(), 4);
    assert!(
        snapshot
            .gaps
            .iter()
            .any(|gap| gap.order == HistoryOrder::Desc && gap.cursor.id == message(8).id.unwrap())
    );
    assert!(snapshot.gaps.iter().all(|gap| gap.cursor.created_at == AT));
    assert!(!snapshot.empty_complete);
}

#[tokio::test]
async fn epoch_only_snapshot_shares_its_vector_and_old_snapshots_survive_one_edit() {
    let channel = channel();
    let mut state = channel.state.lock().await;
    let rows: Vec<_> = (1..=100).rev().map(message).collect();
    merge(&mut state, &rows);
    channel.publish(&state, Generation(1)).unwrap();
    let before = channel.subscribe().unwrap().snapshot;
    channel.generation_changed(Generation(2)).unwrap();
    let restamped = channel.subscribe().unwrap().snapshot;
    assert!(before.messages.ptr_eq(&restamped.messages));
    state
        .model
        .event(
            &papo_api::gateway::KnownEvent::MessageEdit(Box::new(
                papo_api::gateway::MessageEditEvent {
                    id: message(50).id.unwrap(),
                    channel_id: Uuid::nil().to_string(),
                    content: String::new(),
                    edited_at: "2026-01-02T00:00:00.123456Z".into(),
                },
            )),
            2,
        )
        .unwrap();
    channel.publish(&state, Generation(2)).unwrap();
    let edited = channel.subscribe().unwrap().snapshot;
    assert!(before.messages[49].edited_at.is_none());
    assert!(edited.messages[49].edited_at.is_some());
    assert!(Arc::ptr_eq(&before.messages[0], &edited.messages[0]));
    assert_eq!(before.messages.len(), edited.messages.len());
}
