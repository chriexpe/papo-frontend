//! Exact window selection, historical UTC offsets and Arc identity without runtime or network.

use super::*;
use papo_sdk::{Generation, Message, ReadPoint, UnreadCount};

fn message(id: u128, at: &str) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        created_at: Some(at.into()),
        ..Message::default()
    })
}
fn snapshot(messages: Vec<Arc<Message>>) -> HistorySnapshot {
    HistorySnapshot {
        tombstones: imbl::HashSet::new(),
        version: 1,
        generation: Generation::for_test(1),
        messages: messages.into(),
        chunks: vec![],
        gaps: vec![],
        empty_complete: false,
    }
}
fn bounds(raw: &HistorySnapshot, live: bool) -> Bounds {
    Bounds {
        lower: raw
            .messages
            .front()
            .map(|message| key(&message.cursor().unwrap()).unwrap()),
        upper: raw
            .messages
            .last()
            .map(|message| key(&message.cursor().unwrap()).unwrap()),
        live,
    }
}

#[test]
fn read_boundary_update_keeps_every_message_arc_and_replaces_only_marker() {
    let raw = snapshot(vec![
        message(1, "2026-01-01T00:00:00.123456Z"),
        message(2, "2026-01-01T00:00:01Z"),
    ]);
    let bounds = bounds(&raw, true);
    let first = project(
        &raw,
        &bounds,
        Some(&ReadState {
            first_unread: Some(Uuid::from_u128(1)),
            ..ReadState::default()
        }),
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &Vector::new(),
    )
    .unwrap();
    let second = project(
        &raw,
        &bounds,
        Some(&ReadState {
            first_unread: Some(Uuid::from_u128(2)),
            ..ReadState::default()
        }),
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &first,
    )
    .unwrap();
    for id in [1, 2] {
        let find = |items: &Vector<Arc<TimelineItem>>| {
            items.iter().find(|item| matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if *current == Uuid::from_u128(id))).cloned().unwrap()
        };
        assert!(Arc::ptr_eq(&find(&first), &find(&second)));
    }
}

#[test]
fn one_edit_replaces_one_item_and_arrival_is_one_push_back() {
    let a = message(1, "2026-01-01T00:00:01Z");
    let b = message(2, "2026-01-01T00:00:02Z");
    let mut raw = snapshot(vec![a, b]);
    let bounds = bounds(&raw, true);
    let first = project(
        &raw,
        &bounds,
        None,
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &Vector::new(),
    )
    .unwrap();
    raw.messages[1] = message(2, "2026-01-01T00:00:02Z");
    let second = project(
        &raw,
        &bounds,
        None,
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &first,
    )
    .unwrap();
    let diffs = crate::list_diff::between(&first, &second, |item| item.identity());
    assert!(matches!(
        diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 2, .. }]
    ));
    raw.messages.push_back(message(3, "2026-01-01T00:00:03Z"));
    let third = project(
        &raw,
        &bounds,
        None,
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &second,
    )
    .unwrap();
    let diffs = crate::list_diff::between(&second, &third, |item| item.identity());
    assert!(matches!(
        diffs.as_slice(),
        [eyeball_im::VectorDiff::PushBack { .. }]
    ));
}

#[test]
fn utc_offset_rejects_both_twenty_four_hour_boundaries() {
    assert_eq!(crate::UtcOffset::from_seconds_east(-86_400), None);
    assert_eq!(crate::UtcOffset::from_seconds_east(86_400), None);
    for seconds_east in [-86_399, 0, 86_399] {
        assert_eq!(
            crate::UtcOffset::from_seconds_east(seconds_east)
                .unwrap()
                .seconds_east(),
            seconds_east
        );
    }
}

#[test]
fn date_separator_uses_supplied_offset_at_each_message_not_now() {
    let raw = snapshot(vec![
        message(1, "1970-01-01T00:30:00Z"),
        message(2, "1970-01-01T01:30:00Z"),
    ]);
    let offset = |at_us| {
        crate::UtcOffset::from_seconds_east(if at_us < 3_600_000_000 { -3600 } else { 0 }).unwrap()
    };
    let items = project(
        &raw,
        &bounds(&raw, true),
        None,
        [false; 2],
        &offset,
        &Vector::new(),
    )
    .unwrap();
    let days: Vec<_> = items
        .iter()
        .filter_map(|item| {
            if let TimelineItem::DateSeparator { day, .. } = item.as_ref() {
                Some(*day)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(days, [-1, 0]);
}

#[test]
fn focused_end_is_not_live_head_and_deleting_boundary_does_not_shift_window() {
    let mut raw = snapshot(vec![
        message(1, "2026-01-01T00:00:01Z"),
        message(2, "2026-01-01T00:00:02Z"),
        message(3, "2026-01-01T00:00:03Z"),
    ]);
    let bounds = Bounds {
        lower: Some(key(&raw.messages[0].cursor().unwrap()).unwrap()),
        upper: Some(key(&raw.messages[1].cursor().unwrap()).unwrap()),
        live: false,
    };
    raw.messages.remove(0);
    let read = ReadState {
        local_point: Some(ReadPoint {
            created_at_us: 0,
            message_id: Uuid::nil(),
        }),
        unread_count: UnreadCount::AtLeast(2),
        first_unread: Some(Uuid::from_u128(2)),
        ..ReadState::default()
    };
    let items = project(
        &raw,
        &bounds,
        Some(&read),
        [false; 2],
        &|_| crate::UtcOffset::default(),
        &Vector::new(),
    )
    .unwrap();
    let ids: Vec<_> = items
        .iter()
        .filter_map(|item| {
            if let TimelineItem::Message { id, .. } = item.as_ref() {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids, [Uuid::from_u128(2)]);
}

#[test]
fn narrow_focused_window_selects_exact_equal_timestamp_bounds_from_large_history() {
    let raw = snapshot(
        (1..=100_000)
            .map(|id| message(id, "2026-01-01T00:00:00.123456Z"))
            .collect(),
    );
    let bounds = Bounds {
        lower: Some(key(&raw.messages[49_999].cursor().unwrap()).unwrap()),
        upper: Some(key(&raw.messages[50_028].cursor().unwrap()).unwrap()),
        live: false,
    };
    let offsets = std::sync::atomic::AtomicUsize::new(0);
    let items = project(
        &raw,
        &bounds,
        None,
        [false; 2],
        &|_| {
            offsets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::UtcOffset::default()
        },
        &Vector::new(),
    )
    .unwrap();
    let ids: Vec<_> = items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        (50_000..50_030).map(Uuid::from_u128).collect::<Vec<_>>()
    );
    assert_eq!(offsets.load(std::sync::atomic::Ordering::Relaxed), 30);
}
