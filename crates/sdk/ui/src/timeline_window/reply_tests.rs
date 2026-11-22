//! Each reply carries its original as history knows it: shown, held outside the window,
//! deleted or unknown, without fetching and without reading a missing original as deleted.

use super::*;
use crate::timeline_item::ReplyOriginal;
use papo_sdk::{Generation, Message, MessageKey, Uuid};

fn message(id: u128, at: &str, reply_to: Option<u128>) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        created_at: Some(at.into()),
        content: Some(format!("text {id}")),
        reply_to: reply_to.map(|id| Uuid::from_u128(id).to_string()),
        ..Message::default()
    })
}

fn snapshot(messages: Vec<Arc<Message>>, tombstones: &[u128]) -> HistorySnapshot {
    HistorySnapshot {
        tombstones: tombstones.iter().map(|id| Uuid::from_u128(*id)).collect(),
        version: 1,
        generation: Generation::for_test(1),
        messages: messages.into(),
        chunks: vec![],
        gaps: vec![],
        empty_complete: false,
        server_live_complete: false,
        live_arrival_start: None,
    }
}

/// A live window from `lower` on.
fn from(lower: &Arc<Message>) -> Bounds {
    Bounds {
        lower: Some(MessageKey::from_message(lower).unwrap()),
        upper: None,
        live: true,
    }
}

fn items(
    raw: &HistorySnapshot,
    bounds: &Bounds,
    previous: &Vector<Arc<TimelineItem>>,
) -> Vector<Arc<TimelineItem>> {
    project(
        raw,
        bounds,
        None,
        [false; 2],
        &|_| crate::UtcOffset::default(),
        previous,
    )
    .unwrap()
}

fn item(items: &Vector<Arc<TimelineItem>>, id: u128) -> Arc<TimelineItem> {
    items
        .iter()
        .find(|item| {
            matches!(item.as_ref(), TimelineItem::Message { id: current, .. }
                if *current == Uuid::from_u128(id))
        })
        .cloned()
        .unwrap()
}

fn reply_of(items: &Vector<Arc<TimelineItem>>, id: u128) -> Option<ReplyOriginal> {
    match item(items, id).as_ref() {
        TimelineItem::Message { reply, .. } => reply.clone(),
        _ => unreachable!(),
    }
}

#[test]
fn a_reply_to_a_shown_message_shares_its_items_arcs() {
    let original = message(1, "2026-01-01T00:00:01Z", None);
    let raw = snapshot(
        vec![
            Arc::clone(&original),
            message(2, "2026-01-01T00:00:02Z", Some(1)),
        ],
        &[],
    );
    let shown = items(&raw, &from(&original), &Vector::new());
    let Some(ReplyOriginal::Held {
        id,
        message,
        content,
    }) = reply_of(&shown, 2)
    else {
        panic!("the shown original is held");
    };
    assert_eq!(id, Uuid::from_u128(1));
    let TimelineItem::Message {
        message: own,
        content: own_content,
        ..
    } = item(&shown, 1).as_ref().clone()
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&message, &own));
    assert!(Arc::ptr_eq(&content, &own_content));
}

#[test]
fn an_original_before_the_window_is_held_from_history() {
    let original = message(1, "2026-01-01T00:00:01Z", None);
    let reply = message(2, "2026-01-01T00:00:02Z", Some(1));
    let raw = snapshot(vec![original, Arc::clone(&reply)], &[]);
    let shown = items(&raw, &from(&reply), &Vector::new());
    assert!(
        shown.iter().all(|item| !matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == Uuid::from_u128(1))),
        "the original is outside the window"
    );
    let Some(ReplyOriginal::Held {
        message, content, ..
    }) = reply_of(&shown, 2)
    else {
        panic!("history holds the original");
    };
    assert_eq!(message.content.as_deref(), Some("text 1"));
    assert!(!content.blocks.is_empty());
}

#[test]
fn only_a_tombstone_makes_an_original_deleted() {
    let raw = snapshot(
        vec![
            message(2, "2026-01-01T00:00:02Z", Some(1)),
            message(3, "2026-01-01T00:00:03Z", Some(9)),
        ],
        &[1],
    );
    let first = Arc::clone(&raw.messages[0]);
    let shown = items(&raw, &from(&first), &Vector::new());
    assert_eq!(reply_of(&shown, 2), Some(ReplyOriginal::Deleted));
    assert_eq!(
        reply_of(&shown, 3),
        Some(ReplyOriginal::Unknown),
        "an original history never held is unknown, not deleted"
    );
}

#[test]
fn an_empty_reply_is_none_and_an_unparsable_one_unknown() {
    let mut empty = (*message(1, "2026-01-01T00:00:01Z", None)).clone();
    empty.reply_to = Some(String::new());
    let mut garbage = (*message(2, "2026-01-01T00:00:02Z", None)).clone();
    garbage.reply_to = Some("not-an-id".into());
    let raw = snapshot(vec![Arc::new(empty), Arc::new(garbage)], &[]);
    let first = Arc::clone(&raw.messages[0]);
    let shown = items(&raw, &from(&first), &Vector::new());
    assert_eq!(reply_of(&shown, 1), None);
    assert_eq!(reply_of(&shown, 2), Some(ReplyOriginal::Unknown));
}

#[test]
fn editing_an_original_replaces_its_replies_and_nothing_else() {
    let original = message(1, "2026-01-01T00:00:01Z", None);
    let mut raw = snapshot(
        vec![
            Arc::clone(&original),
            message(2, "2026-01-01T00:00:02Z", Some(1)),
            message(3, "2026-01-01T00:00:03Z", None),
        ],
        &[],
    );
    let bounds = from(&original);
    let first = items(&raw, &bounds, &Vector::new());
    let unchanged = items(&raw, &bounds, &first);
    for id in [1, 2, 3] {
        assert!(Arc::ptr_eq(&item(&first, id), &item(&unchanged, id)));
    }
    let mut edited = (*original).clone();
    edited.content = Some("edited".into());
    edited.edited_at = Some("2026-01-01T00:01:00Z".into());
    raw.messages[0] = Arc::new(edited);
    let second = items(&raw, &bounds, &unchanged);
    assert!(!Arc::ptr_eq(&item(&unchanged, 1), &item(&second, 1)));
    assert!(!Arc::ptr_eq(&item(&unchanged, 2), &item(&second, 2)));
    assert!(Arc::ptr_eq(&item(&unchanged, 3), &item(&second, 3)));
    let Some(ReplyOriginal::Held { message, .. }) = reply_of(&second, 2) else {
        panic!("still held");
    };
    assert_eq!(message.content.as_deref(), Some("edited"));
}
