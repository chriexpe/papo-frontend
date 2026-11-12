//! Unowned events wait only for active requests; canonical IDs control replay and generations fence it.

use super::*;
use papo_api::{Uuid, gateway::ReactUpdateEvent, wire::Message};

fn reaction(id: u128, count: i64) -> KnownEvent {
    KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
        message_id: Uuid::from_u128(id).to_string(),
        unicode: Some("x".into()),
        count,
        ..ReactUpdateEvent::default()
    }))
}

fn model(id: u128) -> Model {
    let mut model = Model::default();
    let message = Message {
        id: Some(Uuid::from_u128(id).to_string()),
        channel_id: Some(Uuid::nil().to_string()),
        created_at: Some("2026-01-01T00:00:00.123456Z".into()),
        ..Message::default()
    };
    model.live_message(&message, 1, 1).unwrap();
    model
}

#[test]
fn page_replay_changes_only_its_proven_ids_and_releases_retained_events() {
    let journal = Arc::new(Journal::default());
    let request = journal.request(Generation(1));
    journal.record(Generation(1), &reaction(7, 3));
    let mut owner = model(7);
    let mut unrelated = model(8);
    request.replay(&mut owner).unwrap();
    request.replay(&mut unrelated).unwrap();
    assert_eq!(unrelated.entries.len(), 1);
    assert!(
        unrelated.entries[Uuid::from_u128(8).as_bytes()]
            .message
            .as_ref()
            .unwrap()
            .reactions
            .is_none()
    );
    assert_eq!(
        owner.entries[Uuid::from_u128(7).as_bytes()]
            .message
            .as_ref()
            .unwrap()
            .reactions
            .as_ref()
            .unwrap()[0]
            .count,
        Some(3)
    );
    drop(request);
    assert!(journal.inner.lock().unwrap().events.is_empty());
}

#[test]
fn generation_switch_and_overflow_refuse_stale_pages_without_unbounded_retention() {
    let journal = Arc::new(Journal::default());
    let old = journal.request(Generation(1));
    let current = journal.request(Generation(2));
    assert!(old.replay(&mut model(7)).is_err());
    drop(old);
    for _ in 0..=CAPACITY {
        journal.record(Generation(2), &reaction(7, 3));
    }
    assert_eq!(journal.inner.lock().unwrap().events.len(), CAPACITY);
    assert!(current.replay(&mut model(7)).is_err());
    drop(current);
    assert!(journal.inner.lock().unwrap().events.is_empty());
}

#[test]
fn idle_events_are_not_retained_and_later_requests_do_not_replay_older_totals() {
    let journal = Arc::new(Journal::default());
    journal.record(Generation(1), &reaction(7, 99));
    let early = journal.request(Generation(1));
    journal.record(Generation(1), &reaction(7, 3));
    let later = journal.request(Generation(1));
    let mut owner = model(7);
    later.replay(&mut owner).unwrap();
    assert!(
        owner.entries[Uuid::from_u128(7).as_bytes()]
            .message
            .as_ref()
            .unwrap()
            .reactions
            .is_none()
    );
    drop((early, later));
}

#[test]
fn late_old_generation_registration_cannot_erase_current_request_events() {
    let journal = Arc::new(Journal::default());
    let current = journal.request(Generation(2));
    journal.record(Generation(2), &reaction(7, 3));
    let obsolete = journal.request(Generation(1));
    assert!(obsolete.replay(&mut model(7)).is_err());
    drop(obsolete);
    let mut owner = model(7);
    current.replay(&mut owner).unwrap();
    assert_eq!(
        owner.entries[Uuid::from_u128(7).as_bytes()]
            .message
            .as_ref()
            .unwrap()
            .reactions
            .as_ref()
            .unwrap()[0]
            .count,
        Some(3)
    );
}
