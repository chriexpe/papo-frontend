//! Your unsent messages in the timeline: an echo draws as your message, in
//! your group, with its send's mark where the time goes; the ⚠ of a failed
//! or unconfirmed send is a labelled button; and no state change moves a
//! row.

use std::sync::Arc;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{Generation, Operation, OperationKind, OperationState, OutgoingMessage, Uuid};
use papo_sdk_ui::{TimelineItem, TimelineUpdate};
use rune::Runtime;
use rune::accesskit::Action;

use super::{FRAME_S, Host, message, rect, runtime, until_idle};
use crate::design::{Density, MessageLayout};
use crate::timeline::rows::echo_key;
use crate::timeline::send_mark::mark_key;

/// The test account's user: `key()`'s account is the nil user.
const ME: &str = "00000000-0000-0000-0000-000000000000";

/// 2026-11-20 14:00 UTC, the first test message's time.
const TWO_PM_US: i64 = 1_795_183_200_000_000;

/// Your unsent message with local identity `n`, written `seconds` after
/// 14:00, in `state`.
fn echo(n: u128, seconds: i64, state: OperationState, body: &str) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(n);
    let operation = Operation {
        id,
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(OperationKind::Message(OutgoingMessage {
            content: body.to_owned(),
            ..OutgoingMessage::default()
        })),
        state,
        attempts: 0,
        depends_on: None,
        failure: None,
        message_id: None,
        history_projected: false,
        message: None,
        created_at_us: TWO_PM_US + seconds * 1_000_000,
    };
    Arc::new(TimelineItem::LocalEcho {
        id,
        operation: Arc::new(operation),
        reply: None,
    })
}

/// A message of yours at 14:00, then your echo `n` a minute later.
fn after_mine(state: OperationState) -> Vector<Arc<TimelineItem>> {
    [
        message(1, ME, 0, "first"),
        echo(100, 60, state, "a line of words"),
    ]
    .into_iter()
    .collect()
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

/// Replaces the echo in `rt` with one in `state`, as the SDK's next image.
fn becomes(rt: &mut Runtime<Host>, state: OperationState, version: u64) {
    let next = echo(100, 60, state, "a line of words");
    rt.update(move |host, _tx| {
        host.open.update(
            TimelineUpdate {
                version,
                generation: Generation::for_test(1),
                diffs: vec![VectorDiff::Set {
                    index: 1,
                    value: next,
                }],
            },
            true,
        );
    });
    until_idle(rt);
}

#[test]
fn an_unsent_message_continues_your_group_with_a_clock_for_its_time() {
    let rt = runtime(after_mine(OperationState::Queued), MessageLayout::default());
    let tree = tree(&rt);
    assert!(tree.contains("a line of words"), "{tree}");
    assert!(tree.contains("\"Sending\""), "the clock is named: {tree}");
    let echo_row = format!("/{}/", echo_key(Uuid::from_u128(100)));
    assert!(
        !rt.elements()
            .iter()
            .any(|element| element.path.contains(&echo_row) && element.path.ends_with("/header")),
        "it continues the group: no header"
    );
}

#[test]
fn a_failed_or_unconfirmed_send_offers_its_options() {
    for (state, words) in [
        (OperationState::Failed, "Not sent. Options"),
        (OperationState::Unknown, "May not have arrived. Options"),
    ] {
        let mut rt = runtime(after_mine(state), MessageLayout::default());
        assert!(tree(&rt).contains(words), "{}", tree(&rt));
        rt.accessibility_action(&mark_key(Uuid::from_u128(100)), Action::Click, None);
        rt.frame(FRAME_S);
        assert_eq!(rt.state.controls, ["send-state"]);
    }
}

#[test]
fn no_send_state_moves_a_row_at_any_density() {
    for density in [
        Density::Comfortable,
        Density::Normal,
        Density::Compact,
        Density::ExtraCompact,
    ] {
        let layout = MessageLayout {
            density,
            ..MessageLayout::default()
        };
        let mut rt = runtime(after_mine(OperationState::Queued), layout);
        let row = format!("{}/message", echo_key(Uuid::from_u128(100)));
        let pending = rect(&rt, &row);
        becomes(&mut rt, OperationState::Failed, 2);
        assert_eq!(rect(&rt, &row), pending, "{density:?}: failed");
        becomes(&mut rt, OperationState::Sent, 3);
        assert_eq!(rect(&rt, &row), pending, "{density:?}: sent");
    }
}
