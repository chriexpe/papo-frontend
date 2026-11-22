//! The reading controls over the rows: boundaries that move no row, ⌛
//! and ↓ as the reader's place and the SDK's overlay say, ↓'s arrivals
//! badge, ⌛'s pill with Cancel, and the presses each sends.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Generation, MessageKey, Uuid};
use papo_sdk_ui::{TimelineArrivals, TimelineCatchUp, TimelineItem, TimelineReadingState};
use rune::Runtime;
use rune::accesskit::Action;

use super::{FRAME_S, Host, message, part, rect, runtime, until_idle};
use crate::design::MessageLayout;
use crate::timeline::Seek;

/// `count` messages, each by another author than the one before, so no
/// row continues a group.
fn turns(count: u128) -> Vector<Arc<TimelineItem>> {
    (1..=count)
        .map(|n| {
            let author = if n % 2 == 0 { "ana" } else { "bia" };
            message(n, author, u32::try_from(n).unwrap() * 60, "a line of words")
        })
        .collect()
}

/// `items` with the SDK's unread marker before message `before`.
fn marked(mut items: Vector<Arc<TimelineItem>>, before: u128) -> Vector<Arc<TimelineItem>> {
    let at = items
        .iter()
        .position(|item| matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == Uuid::from_u128(before)))
        .unwrap();
    items.insert(
        at,
        Arc::new(TimelineItem::UnreadMarker {
            before: Uuid::from_u128(before),
        }),
    );
    items
}

fn overlay(arrivals: u64, first: u128, block: Option<(u128, u128)>) -> TimelineReadingState {
    TimelineReadingState {
        version: 1,
        generation: Generation::for_test(1),
        catch_up: block.map(|(start, end)| TimelineCatchUp {
            start_after_id: None,
            start_target_id: Some(Uuid::from_u128(start)),
            end_before_id: Uuid::from_u128(end),
            needs_history: false,
            seeking: false,
        }),
        arrivals: TimelineArrivals {
            count: arrivals,
            oldest_unseen: (arrivals > 0).then(|| {
                MessageKey {
                    created_at_us: 1,
                    message_id: Uuid::from_u128(first),
                }
                .to_cursor()
                .unwrap()
            }),
            live_head: None,
        },
    }
}

fn tops(rt: &Runtime<Host>, ids: std::ops::RangeInclusive<u128>) -> Vec<f32> {
    ids.map(|n| part(rt, n, "message")[1]).collect()
}

fn shows(rt: &Runtime<Host>, key: &str) -> bool {
    rt.find(key).is_some()
}

#[test]
fn the_divider_and_the_moving_marker_move_no_row() {
    let plain = runtime(turns(12), MessageLayout::default());
    let before = tops(&plain, 1..=12);
    let mut rt = runtime(marked(turns(12), 6), MessageLayout::default());
    assert!(shows(&rt, "boundary"), "the divider shows above message 6");
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("New messages"), "it is spoken: {tree}");
    assert_eq!(tops(&rt, 1..=12), before, "drawn over the rows");
    // Read: the SDK's marker moves on, the divider holds.
    rt.update(|host, _tx| {
        let items = marked(turns(12), 10);
        host.open.items = items.clone();
        host.open.keys_version += 1;
        host.open.reading.items_changed(&items, true);
    });
    until_idle(&mut rt);
    assert_eq!(tops(&rt, 1..=12), before, "the marker moved, no row did");
    let divider = rect(&rt, "/boundary");
    let sixth = part(&rt, 6, "message");
    assert!(
        divider[1] < sixth[1] && divider[1] + divider[3] <= sixth[1] + 0.01,
        "over the gap above message 6: {divider:?} vs {sixth:?}"
    );
}

#[test]
fn down_shows_away_from_the_newest_with_its_arrivals() {
    let mut rt = runtime(turns(60), MessageLayout::default());
    assert!(!shows(&rt, "down"), "at the newest, nothing to go to");
    for _ in 0..4 {
        rt.wheel(400.0, 300.0, 500.0);
        rt.frame(FRAME_S);
    }
    until_idle(&mut rt);
    assert!(shows(&rt, "down"), "scrolled up");
    assert!(!shows(&rt, "arrivals"));
    rt.update(|host, _tx| host.open.reading.overlay_changed(overlay(3, 58, None)));
    until_idle(&mut rt);
    assert!(shows(&rt, "arrivals"), "the badge counts the arrivals");
    let tree = rt.accessibility_json().to_string();
    assert!(
        tree.contains("Go to first new message, 3 new messages"),
        "↓ says where it goes and how many came"
    );
    rt.accessibility_action("down", Action::Click, None);
    rt.frame(FRAME_S);
    assert_eq!(rt.state.controls, ["down"]);
}

#[test]
fn the_hourglass_shows_with_a_block_and_its_setting() {
    let mut rt = runtime(turns(20), MessageLayout::default());
    assert!(!shows(&rt, "hourglass"));
    rt.update(|host, _tx| {
        host.open
            .reading
            .overlay_changed(overlay(0, 0, Some((4, 9))));
    });
    until_idle(&mut rt);
    assert!(shows(&rt, "hourglass"));
    assert!(shows(&rt, "down"), "a block is left to clear");
    rt.accessibility_action("hourglass", Action::Click, None);
    rt.frame(FRAME_S);
    assert_eq!(rt.state.controls, ["hourglass"]);
    rt.update(|host, _tx| host.hourglass = false);
    until_idle(&mut rt);
    assert!(!shows(&rt, "hourglass"), "the setting is off");
    assert!(!shows(&rt, "down"), "nothing to clear without it");
}

#[test]
fn while_the_hourglass_pages_its_pill_offers_cancel() {
    let mut rt = runtime(turns(20), MessageLayout::default());
    rt.update(|host, _tx| {
        host.open
            .reading
            .overlay_changed(overlay(0, 0, Some((4, 9))));
        host.open.seek = Some(Seek::new(1).0);
    });
    // The spinner turns while it pages: no rest to wait for.
    for _ in 0..10 {
        rt.frame(FRAME_S);
    }
    assert!(shows(&rt, "seeking"));
    assert!(!shows(&rt, "hourglass"), "one seek at a time");
    rt.accessibility_action("seek-cancel", Action::Click, None);
    rt.frame(FRAME_S);
    assert_eq!(rt.state.controls, ["cancel"]);
}
