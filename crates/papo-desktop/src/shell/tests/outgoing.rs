//! Your unsent messages in the shell: the ⚠ grows Retry and Discard, and
//! Retry asks "Send again?" first when the message may have arrived.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{AccountId, Operation, OperationKind, OperationState, OutgoingMessage, Uuid};
use papo_sdk_ui::TimelineItem;
use rune::Runtime;

use super::navigation::click;
use super::{FRAME_S, Shell, app, server_id, tree};
use crate::timeline::send_mark::mark_key;
use crate::timeline::{OpenTimeline, TimelineKey};

fn unsent(state: OperationState) -> Arc<TimelineItem> {
    let operation = Operation {
        id: Uuid::from_u128(100),
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(OperationKind::Message(OutgoingMessage {
            content: "oi".to_owned(),
            ..OutgoingMessage::default()
        })),
        state,
        attempts: 1,
        depends_on: None,
        failure: None,
        message_id: None,
        history_projected: false,
        message: None,
        created_at_us: 1_795_183_200_000_000,
    };
    Arc::new(TimelineItem::LocalEcho {
        id: operation.id,
        operation: Arc::new(operation),
    })
}

/// The app showing a timeline whose one item is your unsent message.
fn showing(state: OperationState) -> Runtime<Shell> {
    let mut rt = app();
    rt.update(move |shell, _tx| {
        let key = TimelineKey {
            account: AccountId::new(server_id(), Uuid::nil()),
            channel: Uuid::from_u128(7),
        };
        shell.tabs.show(Some(key), false);
        let items: Vector<_> = [unsent(state)].into_iter().collect();
        *shell.tabs.shown_mut().unwrap() = OpenTimeline::loaded(key, items);
    });
    for _ in 0..10 {
        rt.frame(FRAME_S);
    }
    rt
}

#[test]
fn the_mark_grows_retry_and_discard() {
    let mut rt = showing(OperationState::Failed);
    click(&mut rt, &mark_key(Uuid::from_u128(100)));
    let tree = tree(&rt);
    assert!(tree.contains("\"Retry\""), "{tree}");
    assert!(tree.contains("\"Discard\""), "{tree}");
    click(&mut rt, "menu-retry-send");
    assert!(rt.find("menu").is_none(), "the menu closes on a pick");
    assert!(
        rt.find("resend").is_none(),
        "a refused message retries at once"
    );
}

#[test]
fn retrying_a_message_that_may_have_arrived_asks_first() {
    let mut rt = showing(OperationState::Unknown);
    click(&mut rt, &mark_key(Uuid::from_u128(100)));
    click(&mut rt, "menu-retry-send");
    let tree = tree(&rt);
    assert!(tree.contains("Send again?"), "{tree}");
    assert!(tree.contains("could post it twice"), "{tree}");
    click(&mut rt, "resend-cancel");
    assert!(rt.find("resend").is_none());
    assert!(rt.state.resend.is_none());
    click(&mut rt, &mark_key(Uuid::from_u128(100)));
    click(&mut rt, "menu-retry-send");
    click(&mut rt, "resend-confirm");
    assert!(rt.find("resend").is_none(), "Send again closes it");
    assert!(rt.state.resend.is_none());
}

#[test]
fn the_menu_grows_down_from_the_mark() {
    let mut rt = showing(OperationState::Failed);
    let mark = mark_key(Uuid::from_u128(100));
    click(&mut rt, &mark);
    let rect = |key: &str| {
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(key))
            .unwrap()
            .rect
    };
    let (mark, menu) = (rect(&mark), rect("menu"));
    assert!(
        menu[1] >= mark[1] + mark[3],
        "below the mark: {mark:?} {menu:?}"
    );
    assert!(
        (menu[0] - mark[0]).abs() < 1.0,
        "from its left edge: {mark:?} {menu:?}"
    );
}
