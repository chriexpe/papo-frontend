//! Deterministic unit specifications and full-reduction checks for catch-up intervals.
//! Unloaded gap/anchor cases live in `catch_up_gap_tests`; span helpers live in `catch_up_span_tests`.

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
            oldest_anchor: None,
            newest_anchor: None,
            generation: Some(Generation(1)),
            live_generation: Some(Generation(1)),
        }],
        gaps: Vec::new(),
        keys: None,
        empty_complete: false,
        empty_before: None,
        empty_after: None,
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
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    let last = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap()
        .next_bounds;
    assert_eq!(
        state
            .view(&snap, self_user())
            .unwrap()
            .unwrap()
            .start_target,
        Some(msg_id(3))
    );
    let frame: Vec<_> = (3..=8).map(actual_pt).collect();
    let _ = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            last,
            &frame,
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    // Reading [3..=8] closes (after 2, before 9).
    assert!(state.view(&snap, self_user()).unwrap().is_none());
    assert!(state.interval.is_none());
}

#[test]
fn continuous_scrolling_covers_rows_between_frames() {
    let snap = snapshot(1, 10, |_| peer_user());
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    // Frame 1 at [3, 4], flick-scroll continuously to Frame 2 at [7, 8] -> [3..=8] all covered.
    let last = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(3), actual_pt(4)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap()
        .next_bounds;
    assert_eq!(
        state
            .view(&snap, self_user())
            .unwrap()
            .unwrap()
            .start_target,
        Some(msg_id(5))
    );
    state
        .observe_visible(
            &snap,
            self_user(),
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
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(2)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    let last = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap()
        .next_bounds;
    let _ = state
        .observe_visible(
            &snap,
            self_user(),
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
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    // Own messages 3 & 4 skipped when jumping to [5, 6] -> no foreign message skipped.
    state
        .observe_visible(
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.interval.is_none());

    // Arrival batch [7, 8] completely visible -> still no interval.
    let snap2 = snapshot(1, 8, |_| peer_user());
    state
        .observe_visible(
            &snap2,
            self_user(),
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
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    let bottom_last = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(9), actual_pt(10)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap()
        .next_bounds;
    // Scroll up continuously from bottom -> [7, 8] retracts end_before to 7.
    state
        .observe_visible(
            &snap,
            self_user(),
            None,
            bottom_last,
            &[actual_pt(7), actual_pt(8)],
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    assert_eq!(state.interval.as_ref().unwrap().end_before, actual_pt(7));

    // Jump to hourglass start [3, 4] -> start_after advances to 4; bottom progress (7) survives.
    let top_last = state
        .observe_visible(
            &snap,
            self_user(),
            None,
            bottom_last,
            &[actual_pt(3), actual_pt(4)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap()
        .next_bounds;
    assert_eq!(
        state.interval.as_ref().unwrap().start_after,
        Some(actual_pt(4))
    );
    assert_eq!(state.interval.as_ref().unwrap().end_before, actual_pt(7));
    assert_eq!(
        state
            .view(&snap, self_user())
            .unwrap()
            .unwrap()
            .start_target,
        Some(msg_id(5))
    );

    // Reading middle [5, 6] meets retracted end_before 7 -> interval closes.
    state
        .observe_visible(
            &snap,
            self_user(),
            None,
            top_last,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Continuous,
        )
        .unwrap();
    assert!(state.interval.is_none());
}
