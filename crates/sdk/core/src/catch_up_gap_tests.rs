//! Catch-up interval behavior across unloaded gaps, deleted start anchors and legacy millisecond keys.
//! General continuity/retraction tests live in `catch_up_tests`; span helpers live in `catch_up_span_tests`.

use super::tests::{actual_pt, cursor, msg_id, peer_user, self_user, snapshot};
use super::*;
use crate::HistoryChunk;

#[test]
fn unloaded_gap_and_deleted_or_millisecond_anchors_behave_soundly() {
    // Disconnected chunk [1..=3] and [8..=10] with context [5..=6] in between.
    let mut snap = snapshot(1, 10, |_| peer_user());
    let chunk1 = HistoryChunk {
        first: cursor(1),
        last: cursor(3),
        oldest_complete: true,
        newest_complete: false,
        oldest_anchor: None,
        newest_anchor: None,
        generation: Some(Generation(1)),
        live_generation: None,
    };
    let chunk2 = HistoryChunk {
        first: cursor(5),
        last: cursor(6),
        oldest_complete: false,
        newest_complete: false,
        oldest_anchor: None,
        newest_anchor: None,
        generation: Some(Generation(1)),
        live_generation: None,
    };
    let chunk3 = HistoryChunk {
        first: cursor(8),
        last: cursor(10),
        oldest_complete: false,
        newest_complete: true,
        oldest_anchor: None,
        newest_anchor: None,
        generation: Some(Generation(1)),
        live_generation: Some(Generation(1)),
    };
    snap.chunks = vec![chunk1, chunk2, chunk3];
    snap.messages.retain(|m| {
        let p = ReadPoint::from_message(m).unwrap();
        p <= actual_pt(3) || (p >= actual_pt(5) && p <= actual_pt(6)) || p >= actual_pt(8)
    });

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
            &snap,
            self_user(),
            None,
            None,
            &[actual_pt(5), actual_pt(6)],
            VisibilityContinuity::Discontinuous,
        )
        .unwrap();
    assert!(state.interval.is_some());
    assert_eq!(
        state
            .view(&snap, self_user())
            .unwrap()
            .unwrap()
            .start_target,
        Some(msg_id(3))
    );

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
    let ms_view = ms_state.view(&snap, self_user()).unwrap().unwrap();
    // Start target must NOT reopen already-read msg 3; prefix after 3 has gap [4], so needs_history is true.
    assert_eq!(ms_view.start_target, None);
    assert!(ms_view.needs_history);
}

#[test]
fn deleted_start_anchor_does_not_block_visible_catch_up_advance() {
    let mut deleted_snap = snapshot(1, 10, |_| peer_user());
    deleted_snap
        .messages
        .retain(|m| m.id.as_deref() != Some(&msg_id(2).to_string()));
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
            &deleted_snap,
            self_user(),
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
        del_state
            .view(&deleted_snap, self_user())
            .unwrap()
            .unwrap()
            .start_target,
        Some(msg_id(5))
    );
}
