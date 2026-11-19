//! Deterministic held-writer revocation races, server-read suppression under quarantine,
//! and live lookup scheduling budget unit tests.

use super::{
    deliver::{self, DeliveryOutcome},
    live_queue::{LOOKUP_TURN_INTERVAL, LiveState, MAX_EVENTS_BEFORE_LOOKUP, enqueue_lookup},
    policy::PolicyResources,
    tests::{candidate, fixture},
};
use crate::{HistoryClosedReason, MessageKey, StorageError, Uuid};
use std::{sync::mpsc, time::Duration};
use tokio::time::Instant;

fn ledger_count(directory: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(directory.join("state.sqlite")).unwrap();
    connection
        .query_row("SELECT count(*) FROM notification_ledger", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
}

/// Holds the `synchronous=FULL` SQLite writer thread deterministically while `deliver_at`
/// finishes its concurrent read-pool queries (`settings` + `message_read_evidence`) and blocks
/// at `ledger::claim_candidate`.
async fn park_deliver_at_ledger_writer(
    session: &crate::Session,
    generation: crate::Generation,
    item: super::policy::Candidate,
) -> (
    mpsc::Sender<()>,
    tokio::task::JoinHandle<Result<(), StorageError>>,
    tokio::task::JoinHandle<Result<DeliveryOutcome, super::NotificationError>>,
) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let registry = session.client.inner.registry.clone();
    let holder = tokio::spawn(async move {
        registry
            .writer
            .write(move |_tx| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                Ok::<_, StorageError>(())
            })
            .await
    });
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();

    let session_clone = session.clone();
    let deliver_task = tokio::spawn(async move {
        let resources = PolicyResources::from_snapshot(&session_clone.inner.resources.borrow());
        deliver::deliver_at(&session_clone, generation, item, &resources, 0).await
    });

    // Wait until `deliver_at` has acquired `notifications.serial` and suspended behind the
    // held FULL writer (read-pool queries finish immediately without waiting on the writer).
    while session.inner.notifications.serial.try_lock().is_ok() {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !deliver_task.is_finished(),
        "deliver_at must be suspended behind the held FULL ledger writer"
    );
    (release_tx, holder, deliver_task)
}

#[test]
fn revocation_during_held_ledger_writer_suppresses_delivery_in_same_generation() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut item = candidate();
            item.message_point = Some(MessageKey {
                created_at_us: 100,
                message_id: item.message,
            });
            let channel = item.channel;

            let (release_tx, holder, deliver_task) =
                park_deliver_at_ledger_writer(&session, generation, item).await;

            // Revoke the channel within the SAME connection generation after read-evidence
            // evaluation succeeded and while `claim_candidate` is blocked on the FULL writer.
            session.inner.reading.positions.revoke(
                &[channel],
                generation,
                HistoryClosedReason::PermissionDenied,
            );
            let _ = release_tx.send(());
            holder.await.unwrap().unwrap();

            let outcome = deliver_task.await.unwrap().unwrap();
            assert!(matches!(outcome, DeliveryOutcome::Handled(None)));
            assert_eq!(ledger_count(directory.path()), 1);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn quarantine_during_held_ledger_writer_defers_without_ledger_claim() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut item = candidate();
            item.message_point = Some(MessageKey {
                created_at_us: 100,
                message_id: item.message,
            });
            let channel = item.channel;

            let (release_tx, holder, deliver_task) =
                park_deliver_at_ledger_writer(&session, generation, item.clone()).await;

            // Quarantine the channel in the same generation while `claim_candidate` is waiting:
            // must defer BEFORE writing any ledger row so nothing is lost.
            session.inner.reading.positions.quarantine(&[channel]);
            let _ = release_tx.send(());
            holder.await.unwrap().unwrap();

            let outcome = deliver_task.await.unwrap().unwrap();
            assert!(matches!(outcome, DeliveryOutcome::Deferred));
            assert_eq!(ledger_count(directory.path()), 0);

            // Releasing quarantine lets the next attempt deliver and claim once.
            session.inner.reading.positions.release(&[channel]);
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, generation, item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(Some(_))
            ));
            assert_eq!(ledger_count(directory.path()), 1);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn handoff_drops_delivery_when_channel_revoked_after_claim_in_same_generation() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let item = candidate();
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            let DeliveryOutcome::Handled(Some(delivery)) =
                deliver::deliver_at(&session, generation, item.clone(), &resources, 0)
                    .await
                    .unwrap()
            else {
                panic!("expected delivery");
            };

            // Channel revoked after claim and before platform handoff in the same generation.
            session.inner.reading.positions.revoke(
                &[item.channel],
                generation,
                HistoryClosedReason::PermissionDenied,
            );
            deliver::handoff(&session, delivery).unwrap();
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(50),
                    session.notifications().next_delivery(),
                )
                .await
                .is_err()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn confirmed_server_read_suppresses_and_claims_even_when_channel_is_quarantined() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut read_item = candidate();
            read_item.read = true;
            let mut unread_item = candidate();
            unread_item.message = Uuid::from_u128(99);
            unread_item.read = false;

            session
                .inner
                .reading
                .positions
                .quarantine(&[read_item.channel]);
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());

            // Backend-confirmed read=true finalizes immediately without deferring on quarantine.
            assert!(matches!(
                deliver::deliver_at(&session, generation, read_item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
            assert_eq!(ledger_count(directory.path()), 1);

            // Unread candidate on the same quarantined channel still defers before claiming.
            assert!(matches!(
                deliver::deliver_at(&session, generation, unread_item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Deferred
            ));
            assert_eq!(ledger_count(directory.path()), 1);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn live_lookup_scheduler_preempts_after_event_burst_and_paces_turns() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    let generation = session.generation();
    let mut state = LiveState::new(&session);
    let now = Instant::now();

    assert!(!state.lookup_should_preempt_events(now));
    assert!(state.lookup_deadline().is_none());

    enqueue_lookup(&mut state.lookup_queue, generation, candidate(), 1, true).unwrap();
    state.note_lookup_turn(now);
    assert_eq!(state.events_since_lookup, 0);
    assert_eq!(state.lookup_deadline(), Some(now + LOOKUP_TURN_INTERVAL));

    // Before the event burst budget is reached, events are not preempted even once due.
    for _ in 0..(MAX_EVENTS_BEFORE_LOOKUP - 1) {
        state.note_event_while_lookup_pending();
    }
    assert!(!state.lookup_should_preempt_events(now + LOOKUP_TURN_INTERVAL));

    // Once the burst budget is reached, preemption waits for the pacing deadline, then fires.
    state.note_event_while_lookup_pending();
    assert!(!state.lookup_should_preempt_events(now));
    assert!(state.lookup_should_preempt_events(now + LOOKUP_TURN_INTERVAL));

    // Completing a lookup turn resets the burst counter and schedules the next paced slot.
    let later = now + LOOKUP_TURN_INTERVAL;
    state.note_lookup_turn(later);
    assert_eq!(state.events_since_lookup, 0);
    assert_eq!(state.lookup_deadline(), Some(later + LOOKUP_TURN_INTERVAL));

    // Generation change clears deferred queues and pacing state cleanly.
    state.retain_generation(crate::Generation(generation.get() + 1));
    assert!(state.lookup_queue.is_empty());
    assert!(state.lookup_deadline().is_none());
    assert_eq!(state.events_since_lookup, 0);

    client.shutdown().unwrap();
}
