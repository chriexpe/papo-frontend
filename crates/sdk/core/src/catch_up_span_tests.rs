//! Deterministic span hole-preservation and state-machine reduction specs for catch-up intervals.
//! Basic boundary and legacy anchor specs live in `catch_up_tests`.

use super::tests::{actual_pt, cursor, msg_id, peer_user, self_user, snapshot};
use super::*;
use crate::HistoryChunk;

#[test]
fn visible_gap_between_rows_does_not_mark_hole_read() {
    let mut snap = snapshot(1, 10, |_| peer_user());
    snap.chunks = vec![
        HistoryChunk {
            first: cursor(1),
            last: cursor(3),
            oldest_complete: true,
            newest_complete: false,
            generation: Some(Generation(1)),
            live_generation: None,
        },
        HistoryChunk {
            first: cursor(8),
            last: cursor(10),
            oldest_complete: false,
            newest_complete: true,
            generation: Some(Generation(1)),
            live_generation: Some(Generation(1)),
        },
    ];
    snap.messages.retain(|m| {
        let p = ReadPoint::message(m).unwrap();
        p <= actual_pt(3) || p >= actual_pt(8)
    });
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(1)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    // Reporting both sides of a visible gap simultaneously must not mark unloaded [4..=7] read.
    state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(2), actual_pt(8)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    let interval = state
        .interval
        .as_ref()
        .expect("unloaded hole must stay open");
    assert_eq!(interval.start_after, Some(actual_pt(2)));
    assert_eq!(interval.end_before, actual_pt(8));
}

#[test]
fn sparse_discontinuous_report_in_loaded_chunk_leaves_interior_in_catch_up() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();

    // Boundary sparse report [2, 8] against active interval covering 2..=8 must keep 3..=7 unread.
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(1)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(2), actual_pt(8)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    let interval = state
        .interval
        .as_ref()
        .expect("messages 3..=7 were never visible and must remain in catch-up");
    assert_eq!(interval.start_after, Some(actual_pt(2)));
    assert_eq!(interval.end_before, actual_pt(8));
    assert_eq!(interval.covered_spans, vec![]);
    let view = state.view(&index).unwrap().expect("active catch-up view");
    assert_eq!(view.start_target, Some(msg_id(3)));
}

#[test]
fn sparse_interior_and_own_reports_keep_only_foreign_holes_in_catch_up() {
    let snap = snapshot(1, 10, |_| peer_user());
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();

    // Interior sparse report [3, 7] inside (after 1, before 9) must keep 2, 4..=6 and 8 unread.
    let mut interior_state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(1)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    interior_state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(3), actual_pt(7)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    let interior = interior_state
        .interval
        .as_ref()
        .expect("interior omitted rows 4..=6 must remain unread");
    assert_eq!(
        interior.covered_spans,
        vec![
            ReadSpan::normalized(actual_pt(3), actual_pt(3)),
            ReadSpan::normalized(actual_pt(7), actual_pt(7)),
        ]
    );
    assert_eq!(
        interior_state.view(&index).unwrap().unwrap().start_target,
        Some(msg_id(2))
    );
    interior_state
        .observe_visible(
            &index,
            None,
            None,
            &[actual_pt(2)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert_eq!(
        interior_state.view(&index).unwrap().unwrap().start_target,
        Some(msg_id(4))
    );

    // Sparse report spanning only omitted own messages still closes without blocking on self.
    let own_mid = snapshot(1, 10, |i| {
        if (3..=7).contains(&i) {
            self_user()
        } else {
            peer_user()
        }
    });
    let own_index = SnapshotIndex::build(&own_mid, self_user(), &asc).unwrap();
    let mut own_state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(1)),
            end_before: actual_pt(9),
            covered_spans: Vec::new(),
        }),
    };
    own_state
        .observe_visible(
            &own_index,
            None,
            None,
            &[actual_pt(2), actual_pt(8)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(own_state.interval.is_none());
}

#[test]
fn viewport_sequence_matches_naive_unread_set_reduction_including_upward_catch_up() {
    // Deterministic PRNG exercising jumps, continuous up/down scrolls and own messages on 1..=20.
    let mut seed: u64 = 0x5EED_13B0_1234_5678;
    let mut next_u64 = || -> u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let snap = snapshot(
        1,
        20,
        |i| if i % 5 == 0 { self_user() } else { peer_user() },
    );
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(2)),
        interval: None,
    };
    // Naive oracle: exact set of remaining foreign message indices in catch-up interval.
    let mut oracle_unread = std::collections::BTreeSet::<u128>::new();
    let mut seen_max: u128 = 2;
    let mut last_vis: Option<(u128, u128)> = None;
    let mut sdk_last: Option<VisibleBounds> = None;

    for _ in 0..200 {
        let a = u128::from((next_u64() % 20) + 1);
        let b = (a + u128::from(next_u64() % 4)).min(20);
        let continuous = (next_u64() % 2) == 0;
        let cont = if continuous {
            VisibilityContinuity::Continuous
        } else {
            VisibilityContinuity::Discontinuous
        };
        let pts: Vec<_> = (a..=b).map(actual_pt).collect();
        let new_sdk_last = state
            .observe_visible(&index, None, sdk_last, &pts, cont)
            .unwrap()
            .next_bounds;
        sdk_last = new_sdk_last;

        let is_contiguous_scroll = continuous && last_vis.is_some();
        if b > seen_max && !is_contiguous_scroll && a > seen_max {
            let new_set: std::collections::BTreeSet<u128> =
                ((seen_max + 1)..a).filter(|&i| i % 5 != 0).collect();
            if !new_set.is_empty() {
                oracle_unread = new_set;
            }
        }
        let (cov_lo, cov_hi) = if continuous && let Some((plo, phi)) = last_vis {
            (plo.min(a), phi.max(b))
        } else {
            (a, b)
        };
        oracle_unread.retain(|&i| !(cov_lo..=cov_hi).contains(&i));
        seen_max = seen_max.max(b);
        last_vis = Some((a, b));

        let view = state.view(&index).unwrap();
        assert_eq!(view.is_some(), !oracle_unread.is_empty());
        assert_eq!(
            view.and_then(|v| v.start_target),
            oracle_unread.first().copied().map(msg_id)
        );
        assert_eq!(state.seen_to, Some(actual_pt(seen_max)));
    }
}

#[test]
fn empty_asc_successor_does_not_bridge_later_detached_chunk() {
    let mut snap = snapshot(1, 10, |_| peer_user());
    snap.messages.retain(|m| {
        let p = ReadPoint::message(m).unwrap();
        p == actual_pt(1) || p >= actual_pt(8)
    });
    snap.chunks = vec![
        HistoryChunk {
            first: cursor(1),
            last: cursor(1),
            oldest_complete: true,
            newest_complete: false,
            generation: Some(Generation(1)),
            live_generation: None,
        },
        HistoryChunk {
            first: cursor(8),
            last: cursor(10),
            oldest_complete: false,
            newest_complete: true,
            generation: Some(Generation(1)),
            live_generation: Some(Generation(1)),
        },
    ];
    let mut asc = AscSuccessors::new();
    asc.insert(msg_id(1), None);
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    assert!(
        !index
            .is_contiguous(Some(actual_pt(1)), actual_pt(9))
            .unwrap()
    );

    // Only when no loaded entry exists after `cursor` (e.g., tail deleted after `cursor`)
    // can an empty `Asc(cursor)` proof confirm an empty `(cursor, deleted_end]` interval.
    let mut empty_tail = snapshot(1, 3, |_| peer_user());
    empty_tail
        .messages
        .retain(|m| ReadPoint::message(m).unwrap() == actual_pt(1));
    empty_tail.chunks.clear();
    let tail_index = SnapshotIndex::build(&empty_tail, self_user(), &asc).unwrap();
    assert!(
        tail_index
            .is_contiguous(Some(actual_pt(1)), actual_pt(3))
            .unwrap()
    );
}

#[test]
fn continuous_scroll_bridges_only_seam_not_internal_chunk_gap() {
    let mut snap = snapshot(1, 10, |_| peer_user());
    snap.messages.retain(|m| {
        let p = ReadPoint::message(m).unwrap();
        p <= actual_pt(3) || p >= actual_pt(8)
    });
    snap.chunks = vec![
        HistoryChunk {
            first: cursor(1),
            last: cursor(3),
            oldest_complete: true,
            newest_complete: false,
            generation: Some(Generation(1)),
            live_generation: None,
        },
        HistoryChunk {
            first: cursor(8),
            last: cursor(10),
            oldest_complete: false,
            newest_complete: true,
            generation: Some(Generation(1)),
            live_generation: Some(Generation(1)),
        },
    ];
    let asc = AscSuccessors::new();
    let index = SnapshotIndex::build(&snap, self_user(), &asc).unwrap();
    let mut state = CatchUpState {
        seen_to: Some(actual_pt(10)),
        interval: Some(CatchUpInterval {
            start_after: Some(actual_pt(1)),
            end_before: actual_pt(10),
            covered_spans: Vec::new(),
        }),
    };
    let last = state
        .observe_visible(
            &index,
            None,
            Some(VisibleBounds {
                min: actual_pt(1),
                max: actual_pt(2),
            }),
            &[actual_pt(3), actual_pt(8)],
            VisibilityContinuity::Continuous,
        )
        .unwrap()
        .next_bounds;
    assert_eq!(
        last,
        Some(VisibleBounds {
            min: actual_pt(3),
            max: actual_pt(8)
        })
    );
    let gap = state
        .interval
        .as_ref()
        .expect("unloaded hole [4..=7] must stay open");
    assert!(
        !index
            .no_unseen_foreign(Some(actual_pt(1)), actual_pt(10), &gap.covered_spans)
            .unwrap()
    );
}
