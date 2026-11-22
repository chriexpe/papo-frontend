//! Your edits and deletions in flight on their messages, and the reply
//! line (`docs/design.md` §11): an edit's text at once with a clock, ⚠ and
//! its menu when it failed; a deletion at half opacity, then a line with
//! Try again and Dismiss when it failed; a reply's original, deleted or not
//! available, with the elbow or Extra compact's icon.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, MessageTarget, Operation, OperationKind, OperationState, Uuid};
use papo_sdk_ui::{ReplyOriginal, TimelineItem};
use rune::accesskit::Action;

use super::{Host, find_part, message, runtime, until_idle};
use crate::design::{Density, MessageLayout};
use crate::timeline::change_marks::change_mark_key;

const ME: &str = "00000000-0000-0000-0000-000000000000";

fn change(n: u128, kind: OperationKind, state: OperationState) -> Arc<Operation> {
    Arc::new(Operation {
        id: Uuid::from_u128(1000 + n),
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(kind),
        state,
        attempts: 0,
        depends_on: None,
        failure: None,
        message_id: Some(Uuid::from_u128(n)),
        history_projected: false,
        message: None,
        created_at_us: 0,
    })
}

fn edit(n: u128, state: OperationState) -> Arc<Operation> {
    let kind = OperationKind::Edit {
        target: MessageTarget::Remote(Uuid::from_u128(n)),
        content: "edited words".to_owned(),
        embeds: None,
    };
    change(n, kind, state)
}

fn delete(n: u128, state: OperationState) -> Arc<Operation> {
    let kind = OperationKind::Delete {
        target: MessageTarget::Remote(Uuid::from_u128(n)),
    };
    change(n, kind, state)
}

/// `item` (a message) with `pending` and `reply` on it.
fn with(
    item: &Arc<TimelineItem>,
    pending: Option<Arc<Operation>>,
    reply: Option<ReplyOriginal>,
) -> Arc<TimelineItem> {
    let TimelineItem::Message {
        id,
        message,
        content,
        ..
    } = item.as_ref()
    else {
        unreachable!()
    };
    let mut message = Message::clone(message);
    message.reply_to = reply.as_ref().map(|_| Uuid::from_u128(1).to_string());
    Arc::new(TimelineItem::Message {
        id: *id,
        message: Arc::new(message),
        content: Arc::clone(content),
        reply,
        pending,
    })
}

fn tree(rt: &rune::Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

#[test]
fn an_edit_on_its_way_shows_its_text_and_a_clock_in_the_times_place() {
    let items: Vector<_> = [
        message(1, ME, 0, "first"),
        with(
            &message(2, ME, 60, "original words"),
            Some(edit(2, OperationState::Sending)),
            None,
        ),
    ]
    .into_iter()
    .collect();
    let rt = runtime(items, MessageLayout::default());
    let tree = tree(&rt);
    assert!(tree.contains("edited words"), "{tree}");
    assert!(!tree.contains("original words"), "{tree}");
    assert!(rt.find(&change_mark_key(Uuid::from_u128(1002))).is_some());
}

#[test]
fn a_failed_edit_s_warning_opens_its_menu() {
    let items: Vector<_> = [
        message(1, ME, 0, "first"),
        with(
            &message(2, ME, 60, "original words"),
            Some(edit(2, OperationState::Failed)),
            None,
        ),
    ]
    .into_iter()
    .collect();
    let mut rt = runtime(items, MessageLayout::default());
    rt.accessibility_action(&change_mark_key(Uuid::from_u128(1002)), Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(rt.state.controls, ["change-mark"]);
}

#[test]
fn a_deletion_dims_its_message_and_a_failed_one_offers_try_again_and_dismiss() {
    let items: Vector<_> = [
        message(1, ME, 0, "first"),
        with(
            &message(2, ME, 60, "going"),
            Some(delete(2, OperationState::Sending)),
            None,
        ),
        with(
            &message(3, ME, 120, "stays"),
            Some(delete(3, OperationState::Failed)),
            None,
        ),
    ]
    .into_iter()
    .collect();
    let mut rt = runtime(items, MessageLayout::default());
    let dimmed = find_part(&rt, 2, "message").unwrap();
    assert!((dimmed.opacity - 0.5).abs() < 0.01, "{}", dimmed.opacity);
    assert!(find_part(&rt, 3, "delete-failed").is_some());
    rt.accessibility_action("delete-retry", Action::Click, None);
    rt.accessibility_action("delete-dismiss", Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(rt.state.controls, ["change-retried", "change-discarded"]);
}

#[test]
fn a_reply_line_names_its_original_or_says_it_is_gone_and_starts_a_group() {
    let original = message(1, "lia", 0, "is the build green?");
    let held = match original.as_ref() {
        TimelineItem::Message {
            id,
            message,
            content,
            ..
        } => ReplyOriginal::Held {
            id: *id,
            message: Arc::clone(message),
            content: Arc::clone(content),
        },
        _ => unreachable!(),
    };
    for density in [Density::Normal, Density::ExtraCompact] {
        let layout = MessageLayout {
            density,
            ..MessageLayout::default()
        };
        let items: Vector<_> = [
            Arc::clone(&original),
            with(&message(2, ME, 60, "yes"), None, Some(held.clone())),
            with(
                &message(3, ME, 120, "and"),
                None,
                Some(ReplyOriginal::Deleted),
            ),
            with(
                &message(4, ME, 180, "also"),
                None,
                Some(ReplyOriginal::Unknown),
            ),
        ]
        .into_iter()
        .collect();
        let rt = runtime(items, layout);
        let tree = tree(&rt);
        assert!(tree.contains("is the build green?"), "{tree}");
        assert!(tree.contains("Original message deleted"), "{tree}");
        assert!(tree.contains("Reply to an earlier message"), "{tree}");
        for n in [2, 3, 4] {
            assert!(find_part(&rt, n, "reply-line").is_some(), "{density:?} {n}");
            // A reply starts its own group: its header shows.
            let header = if density == Density::ExtraCompact {
                "name"
            } else {
                "header"
            };
            assert!(find_part(&rt, n, header).is_some(), "{density:?} {n}");
        }
        if density != Density::ExtraCompact {
            // The elbow's leg reaches the avatar, which sits below the
            // line's box.
            let elbow = find_part(&rt, 2, "elbow").unwrap().rect;
            let avatar = find_part(&rt, 2, "avatar").unwrap().rect;
            let gap = avatar[1] - (elbow[1] + elbow[3]);
            assert!(
                gap.abs() < 0.5,
                "{density:?}: {gap} between elbow and avatar"
            );
        }
    }
}

#[test]
fn a_second_edit_on_its_way_shows_its_own_text() {
    let mine = message(2, ME, 60, "original words");
    let items: Vector<_> = [
        message(1, ME, 0, "first"),
        with(&mine, Some(edit(2, OperationState::Queued)), None),
    ]
    .into_iter()
    .collect();
    let mut rt = runtime(items, MessageLayout::default());
    assert!(tree(&rt).contains("edited words"));
    let again = OperationKind::Edit {
        target: MessageTarget::Remote(Uuid::from_u128(2)),
        content: "edited again".to_owned(),
        embeds: None,
    };
    let next = with(&mine, Some(change(2, again, OperationState::Queued)), None);
    rt.update(move |host, _tx| {
        host.open.update(
            papo_sdk_ui::TimelineUpdate {
                version: 2,
                generation: papo_sdk::Generation::for_test(1),
                diffs: vec![eyeball_im::VectorDiff::Set {
                    index: 1,
                    value: next,
                }],
            },
            true,
        );
    });
    until_idle(&mut rt);
    let tree = tree(&rt);
    assert!(tree.contains("edited again"), "{tree}");
    assert!(!tree.contains("edited words"), "{tree}");
}
