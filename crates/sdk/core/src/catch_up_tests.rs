//! Deterministic unit specifications and full-reduction checks for catch-up intervals.
//! Durability and network sync tests live beside `read_points` and integration suites.

use super::*;
use crate::{HistoryChunk, HistorySnapshot, Message};
use papo_api::HistoryCursor;
use std::sync::Arc;

pub(super) fn self_user() -> Uuid {
    Uuid::from_u128(1000)
}

pub(super) fn peer_user() -> Uuid {
    Uuid::from_u128(2000)
}

pub(super) fn msg_id(index: u128) -> Uuid {
    Uuid::from_u128(index)
}

pub(super) fn cursor(index: u128) -> HistoryCursor {
    HistoryCursor {
        created_at: format!("2026-01-01T00:00:00.{index:06}Z"),
        id: msg_id(index).to_string(),
    }
}

fn make_message(index: u128, author: Uuid) -> Arc<Message> {
    Arc::new(Message {
        id: Some(msg_id(index).to_string()),
        author_id: Some(author.to_string()),
        created_at: Some(cursor(index).created_at),
        ..Message::default()
    })
}

pub(super) fn snapshot(start: u128, end: u128, authors: impl Fn(u128) -> Uuid) -> HistorySnapshot {
    let messages: imbl::Vector<_> = (start..=end).map(|i| make_message(i, authors(i))).collect();
    HistorySnapshot {
        tombstones: imbl::HashSet::new(),
        version: 1,
        generation: Generation(1),
        messages,
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
        server_live_complete: true,
        live_arrival_start: None,
    }
}

pub(super) fn actual_pt(index: u128) -> ReadPoint {
    ReadPoint::parse(Some(&cursor(index).created_at), Some(&cursor(index).id)).unwrap()
}

#[test]
fn reading_through_the_gap_retires_the_hourglass() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    let (_, last) = state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.view(&index).unwrap().has_catch_up);
    assert_eq!(state.view(&index).unwrap().start_target, Some(msg_id(3)));
    let frame: Vec<_> = (3..=8).map(actual_pt).collect();
    let _ = state
        .observe_visible(&index, None, last, &frame, VisibilityContinuity::Continuous)
        .unwrap();
    // Reading [3..=8] closes (after 2, before 9).
    let view = state.view(&index).unwrap();
    assert!(!view.has_catch_up);
    assert!(state.interval.is_none());
}

#[test]
fn continuous_scrolling_covers_rows_between_frames() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    // Frame 1 at [3, 4], flick-scroll continuously to Frame 2 at [7, 8] -> [3..=8] all covered.
    let (_, last) = state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(3), actual_pt(4)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert_eq!(state.view(&index).unwrap().start_target, Some(msg_id(5)));
    state
        .observe_visible(
            &index,
            None,
            last,
            &[actual_pt(7), actual_pt(8)],
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    assert!(state.interval.is_none());
}

#[test]
fn a_jump_does_not_cover_what_it_flew_over() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    let (_, last) = state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    let _ = state
        .observe_visible(
            &index,
            None,
            last,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    let interval = state.interval.as_ref().unwrap();
    assert_eq!(interval.start_after, Some(actual_pt(2)));
    assert_eq!(interval.end_before, actual_pt(9));
    assert_eq!(
        interval.covered_spans,
        vec![ReadSpan {
            start: actual_pt(5),
            end: actual_pt(6),
        }]
    );
}

#[test]
fn a_batch_of_arrivals_at_the_bottom_and_own_messages_leave_no_hourglass() {
    let snap = snapshot(1, 6, |i| {
        if i == 3 || i == 4 {
            self_user()
        } else {
            peer_user()
        }
    });
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    // Own messages 3 & 4 skipped when jumping to [5, 6] -> no foreign message skipped.
    state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.interval.is_none());

    // Arrival batch [7, 8] completely visible -> still no interval.
    let snap2 = snapshot(1, 8, |_| peer_user());
    let index2 = SnapshotIndex::build(&snap2, self_user(), &asc).unwrap();
    state
        .observe_visible(
            &index2,
            None,
            None,
            &[actual_pt(7), actual_pt(8)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.interval.is_none());
}

#[test]
fn hourglass_range_shrinks_from_top_and_bottom_and_meets_in_middle() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    let (_, bottom_last) = state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    // Scroll up continuously from bottom -> [7, 8] retracts end_before to 7.
    state
        .observe_visible(
            &index,
            None,
            bottom_last,
            &[actual_pt(7), actual_pt(8)],
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    assert_eq!(state.interval.as_ref().unwrap().end_before, actual_pt(7));

    // Jump to hourglass start [3, 4] -> start_after advances to 4; bottom progress (7) survives.
    let (_, top_last) = state
        .observe_visible(
            &index,
            None,
            bottom_last,
            &[actual_pt(3), actual_pt(4)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert_eq!(
        state.interval.as_ref().unwrap().start_after,
        Some(actual_pt(4))
    );
    assert_eq!(state.interval.as_ref().unwrap().end_before, actual_pt(7));
    assert_eq!(state.view(&index).unwrap().start_target, Some(msg_id(5)));

    // Reading middle [5, 6] meets retracted end_before 7 -> interval closes.
    state
        .observe_visible(
            &index,
            None,
            top_last,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    assert!(state.interval.is_none());
}

#[test]
fn unloaded_gap_and_deleted_or_millisecond_anchors_behave_soundly() {
    // Disconnected chunk [1..=3] and [8..=10] with context [5..=6] in between.
    let mut snap = snapshot(1, 10, |_| peer_user());
    let chunk1 = HistoryChunk {
        first: cursor(1),
        last: cursor(3),
        oldest_complete: true,
        newest_complete: false,
        generation: Some(Generation(1)),
        live_generation: None,
    };
    let chunk2 = HistoryChunk {
        first: cursor(5),
        last: cursor(6),
        oldest_complete: false,
        newest_complete: false,
        generation: Some(Generation(1)),
        live_generation: None,
    };
    let chunk3 = HistoryChunk {
        first: cursor(8),
        last: cursor(10),
        oldest_complete: false,
        newest_complete: true,
        generation: Some(Generation(1)),
        live_generation: Some(Generation(1)),
    };
    snap.chunks = vec![chunk1, chunk2, chunk3];
    snap.messages.retain(|m| {
        let p = ReadPoint::message(m).unwrap();
        p <= actual_pt(3) || (p >= actual_pt(5) && p <= actual_pt(6)) || p >= actual_pt(8)
    });
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();

    // Context page [5, 6] does not close unloaded interval (after 2, before 9).
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.interval.is_some());
    assert_eq!(state.view(&index).unwrap().start_target, Some(msg_id(3)));

    // Legacy millisecond truncated anchor for msg 3 (.000Z while message has .000003Z).
    let ms_truncated = ReadPoint {
        created_at_us: 1_767_225_600_000_000,
        message_id: msg_id(3),
    };
    let ms_state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(ms_truncated),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    let ms_view = ms_state.view(&index).unwrap();
    // Start target must NOT reopen already-read msg 3; prefix after 3 has gap [4], so needs_history is true.
    assert_eq!(ms_view.start_target, None);
    assert!(ms_view.needs_history);

    // Deleting msg 2 on continuous [1..=10] does not block reading 3..4.
    let mut deleted_snap = snapshot(1, 10, |_| peer_user());
    deleted_snap
        .messages
        .retain(|m| m.id.as_deref() != Some(&msg_id(2).to_string()));
    let del_index = SnapshotIndex::build(&deleted_snap, self_user(), &asc).unwrap();
    let mut del_state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    del_state
        .observe_visible(
            &del_index,
            None,
            None,
            &[actual_pt(3), actual_pt(4)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert_eq!(
        del_state.interval.as_ref().unwrap().start_after,
        Some(actual_pt(4))
    );
    assert_eq!(
        del_state.view(&del_index).unwrap().start_target,
        Some(msg_id(5))
    );
}
