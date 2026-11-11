//! Deterministic specifications for ordered reading and incomplete cached coverage; no legacy catch-up state.

use super::*;
use crate::{HistoryChunk, HistorySnapshot};
use papo_api::HistoryCursor;

fn point(index: u128) -> ReadPoint {
    ReadPoint {
        created_at_us: i64::try_from(index).unwrap(),
        message_id: Uuid::from_u128(index),
    }
}
fn cursor(index: u128) -> HistoryCursor {
    HistoryCursor {
        created_at: format!("2026-01-01T00:00:00.{index:06}Z"),
        id: Uuid::from_u128(index).to_string(),
    }
}
fn snapshot(start: u128, end: u128) -> HistorySnapshot {
    HistorySnapshot {
        version: 1,
        generation: Generation(1),
        messages: (start..=end)
            .map(|index| {
                Arc::new(Message {
                    id: Some(Uuid::from_u128(index).to_string()),
                    created_at: Some(cursor(index).created_at),
                    ..Message::default()
                })
            })
            .collect(),
        chunks: vec![HistoryChunk {
            first: cursor(start),
            last: cursor(end),
            oldest_complete: true,
            newest_complete: true,
            generation: Some(Generation(1)),
            live_generation: Some(Generation(1)),
        }],
        gaps: Vec::new(),
        empty_complete: false,
    }
}
fn actual_point(index: u128) -> ReadPoint {
    ReadPoint::parse(Some(&cursor(index).created_at), Some(&cursor(index).id)).unwrap()
}

#[test]
fn full_microsecond_precision_and_uuid_tiebreaker_survive_round_trip() {
    let point = ReadPoint::parse(
        Some("2026-01-01T00:00:00.123456Z"),
        Some(&Uuid::from_u128(7).to_string()),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&point).unwrap();
    assert_eq!(serde_json::from_slice::<ReadPoint>(&bytes).unwrap(), point);
    assert!(
        ReadPoint {
            message_id: Uuid::from_u128(8),
            ..point
        } > point
    );
    assert!(
        ReadPoint {
            created_at_us: point.created_at_us + 1,
            message_id: Uuid::nil()
        } > point
    );
}

#[test]
fn non_backend_nanosecond_precision_is_rejected_not_rounded() {
    assert!(
        ReadPoint::parse(
            Some("2026-01-01T00:00:00.123456789Z"),
            Some(&Uuid::nil().to_string())
        )
        .is_err()
    );
}

#[test]
fn uploaded_and_local_points_are_independently_monotonic_inputs() {
    let mut state = ReadState {
        local_point: Some(point(3)),
        uploaded_point: Some(point(2)),
        ..ReadState::default()
    };
    assert_eq!(state.pending(), Some(point(3)));
    state.uploaded_point = Some(point(5));
    assert_eq!(state.point(), Some(point(5)));
    assert_eq!(state.pending(), None);
}

#[test]
fn contiguous_current_coverage_reports_exact_count_and_first_unread() {
    let mut state = ReadState {
        local_point: Some(actual_point(3)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 10), None).unwrap();
    assert_eq!(
        (state.unread_count, state.first_unread),
        (UnreadCount::Exact(7), Some(Uuid::from_u128(4)))
    );
}

#[test]
fn point_older_than_retained_history_is_a_lower_bound() {
    let mut state = ReadState {
        local_point: Some(actual_point(1)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(4, 10), None).unwrap();
    assert_eq!(state.unread_count, UnreadCount::AtLeast(7));
}

#[test]
fn unknown_middle_is_never_an_exact_unread_count() {
    let mut history = snapshot(1, 10);
    let mut older = history.chunks[0].clone();
    older.last = cursor(3);
    older.newest_complete = false;
    history.chunks[0].first = cursor(8);
    history.chunks[0].oldest_complete = false;
    history.chunks.insert(0, older);
    history.messages.retain(|message| {
        let point = ReadPoint::message(message).unwrap();
        point <= actual_point(3) || point >= actual_point(8)
    });
    let mut state = ReadState {
        local_point: Some(actual_point(2)),
        ..ReadState::default()
    };
    counts::update(&mut state, &history, None).unwrap();
    assert_eq!(state.unread_count, UnreadCount::AtLeast(4));
}

#[test]
fn read_cursor_in_newest_run_does_not_inherit_an_older_unrelated_gap() {
    let mut history = snapshot(8, 10);
    history.chunks[0].oldest_complete = false;
    let mut state = ReadState {
        local_point: Some(actual_point(9)),
        ..ReadState::default()
    };
    counts::update(&mut state, &history, None).unwrap();
    assert_eq!(state.unread_count, UnreadCount::Exact(1));
}

#[test]
fn a_new_arrival_behind_the_viewport_stays_unread() {
    let mut state = ReadState {
        local_point: Some(actual_point(10)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 11), None).unwrap();
    assert_eq!(
        (state.unread_count, state.first_unread),
        (UnreadCount::Exact(1), Some(Uuid::from_u128(11)))
    );
}

#[test]
fn focused_window_end_does_not_acknowledge_the_live_head() {
    let mut state = ReadState {
        local_point: Some(actual_point(4)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 10), None).unwrap();
    assert_eq!(state.unread_count, UnreadCount::Exact(6));
}

#[test]
fn resolved_remote_mark_clears_unread_without_fabricating_local_reports() {
    let mut state = ReadState {
        local_point: Some(actual_point(4)),
        uploaded_point: Some(actual_point(10)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 10), None).unwrap();
    assert_eq!(
        (state.local_point, state.unread_count, state.pending()),
        (Some(actual_point(4)), UnreadCount::Exact(0), None)
    );
}

#[test]
fn deleted_anchor_keeps_its_order_and_does_not_wedge_successors() {
    let mut history = snapshot(1, 10);
    history
        .messages
        .retain(|message| message.id.as_deref() != Some(&Uuid::from_u128(3).to_string()));
    let mut state = ReadState {
        local_point: Some(actual_point(3)),
        ..ReadState::default()
    };
    counts::update(&mut state, &history, None).unwrap();
    assert_eq!(
        (state.unread_count, state.first_unread),
        (UnreadCount::Exact(7), Some(Uuid::from_u128(4)))
    );
}

#[test]
fn generation_restamping_invalidates_old_live_edge_proof() {
    let mut history = snapshot(1, 10);
    history.generation = Generation(2);
    let mut state = ReadState {
        local_point: Some(actual_point(3)),
        ..ReadState::default()
    };
    counts::update(&mut state, &history, None).unwrap();
    assert_eq!(state.unread_count, UnreadCount::AtLeast(7));
}

#[test]
fn unresolved_remote_mark_does_not_invent_a_positive_count_or_first_unread() {
    let mut state = ReadState {
        local_point: Some(actual_point(3)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 10), None).unwrap();
    counts::unresolved(&mut state);
    assert_eq!(
        (state.unread_count, state.first_unread),
        (UnreadCount::AtLeast(0), None)
    );
}

#[test]
fn metadata_newer_than_cache_is_a_real_unread_lower_bound_not_an_exact_total() {
    let mut state = ReadState {
        local_point: Some(actual_point(10)),
        ..ReadState::default()
    };
    counts::update(&mut state, &snapshot(1, 10), Some(actual_point(30))).unwrap();
    assert_eq!(
        (state.unread_count, state.first_unread),
        (UnreadCount::AtLeast(1), None)
    );
}
