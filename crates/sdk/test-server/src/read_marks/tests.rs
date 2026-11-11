//! Owns pure pinned read-state regressions; leaves sockets and real clocks to integration tests.

use super::*;
use crate::{HistorySeed, history::History, resources::Resources};

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const FIRST: &str = "20000000-0000-4000-8000-000000000001";
const SECOND: &str = "20000000-0000-4000-8000-000000000002";
const THIRD: &str = "20000000-0000-4000-8000-000000000003";
const ACK: &str = "2026-02-01T00:00:00Z";

fn fixture() -> (Resources, History) {
    crate::history_seed::prepare(&[HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: "30000000-0000-4000-8000-000000000001".into(),
        first_message_id: FIRST.into(),
        count: 3,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 2,
    }])
    .unwrap()
}

fn mark<'a>(resources: &'a Resources, user: &str) -> Option<&'a (String, String)> {
    resources.read_marks.get(&(user.into(), CHANNEL.into()))
}

#[test]
fn equal_timestamp_advances_by_uuid_and_not_acknowledgement_time() {
    let (mut resources, history) = fixture();
    advance(&mut resources, &history, "reader", CHANNEL, FIRST, ACK).unwrap();
    advance(
        &mut resources,
        &history,
        "reader",
        CHANNEL,
        SECOND,
        "2026-01-02T00:00:00Z",
    )
    .unwrap();
    assert_eq!(
        mark(&resources, "reader"),
        Some(&(SECOND.into(), "2026-01-02T00:00:00Z".into()))
    );
}

#[test]
fn duplicate_and_older_reads_do_not_refresh_acknowledgement_time() {
    let (mut resources, history) = fixture();
    advance(&mut resources, &history, "reader", CHANNEL, SECOND, ACK).unwrap();
    for id in [FIRST, SECOND] {
        assert_eq!(
            advance(
                &mut resources,
                &history,
                "reader",
                CHANNEL,
                id,
                "2026-03-01T00:00:00Z"
            ),
            Ok(false)
        );
    }
    assert_eq!(
        mark(&resources, "reader"),
        Some(&(SECOND.into(), ACK.into()))
    );
}

#[test]
fn newer_microsecond_advances_even_with_a_lower_uuid() {
    let (mut resources, mut history) = fixture();
    let mut later = history
        .message(Uuid::parse_str(THIRD).unwrap())
        .unwrap()
        .clone();
    later.id = Some("00000000-0000-4000-8000-000000000001".into());
    history.insert(later.clone()).unwrap();
    advance(&mut resources, &history, "reader", CHANNEL, SECOND, ACK).unwrap();
    assert_eq!(
        advance(
            &mut resources,
            &history,
            "reader",
            CHANNEL,
            later.id.as_deref().unwrap(),
            ACK
        ),
        Ok(true)
    );
}

#[test]
fn deleted_previous_message_compares_as_epoch_nil_uuid() {
    let (mut resources, mut history) = fixture();
    advance(&mut resources, &history, "reader", CHANNEL, THIRD, ACK).unwrap();
    history.remove(Uuid::parse_str(THIRD).unwrap());
    advance(&mut resources, &history, "reader", CHANNEL, FIRST, ACK).unwrap();
    assert_eq!(
        mark(&resources, "reader").map(|mark| mark.0.as_str()),
        Some(FIRST)
    );
}

#[test]
fn deleted_mark_fallback_does_not_advance_to_a_pre_epoch_message() {
    let (mut resources, mut history) = fixture();
    advance(&mut resources, &history, "reader", CHANNEL, THIRD, ACK).unwrap();
    history.remove(Uuid::parse_str(THIRD).unwrap());
    let mut before_epoch = history
        .message(Uuid::parse_str(FIRST).unwrap())
        .unwrap()
        .clone();
    before_epoch.id = Some(THIRD.into());
    before_epoch.created_at = Some("1969-12-31T23:59:59Z".into());
    history.insert(before_epoch).unwrap();
    resources.read_marks.insert(
        ("reader".into(), CHANNEL.into()),
        ("20000000-0000-4000-8000-000000000099".into(), ACK.into()),
    );
    assert_eq!(
        advance(&mut resources, &history, "reader", CHANNEL, THIRD, ACK),
        Ok(false)
    );
}

#[test]
fn missing_mark_accepts_a_pre_epoch_message_without_fallback() {
    let (mut resources, mut history) = fixture();
    let mut before_epoch = history.remove(Uuid::parse_str(FIRST).unwrap()).unwrap();
    before_epoch.created_at = Some("1969-12-31T23:59:59Z".into());
    history.insert(before_epoch).unwrap();
    assert_eq!(
        advance(&mut resources, &history, "reader", CHANNEL, FIRST, ACK),
        Ok(true)
    );
}

#[test]
fn marks_are_account_scoped() {
    let (mut resources, history) = fixture();
    advance(&mut resources, &history, "a", CHANNEL, THIRD, ACK).unwrap();
    advance(&mut resources, &history, "b", CHANNEL, FIRST, ACK).unwrap();
    assert_eq!(
        mark(&resources, "b").map(|mark| mark.0.as_str()),
        Some(FIRST)
    );
}

#[test]
fn permission_and_message_validation_never_mutate_marks() {
    let (mut resources, mut history) = fixture();
    history.set_readable("reader", CHANNEL, false);
    assert_eq!(
        advance(&mut resources, &history, "reader", CHANNEL, FIRST, ACK),
        Err(ReadFailure::Forbidden)
    );
    history.set_readable("reader", CHANNEL, true);
    assert_eq!(
        advance(
            &mut resources,
            &history,
            "reader",
            CHANNEL,
            "20000000-0000-4000-8000-000000000099",
            ACK
        ),
        Err(ReadFailure::Missing)
    );
    assert!(resources.read_marks.is_empty());
}
