//! The reader's place through density changes, paging, edits and reflow
//! over 10,000 messages; the anchor read back; paging; idle.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use rune::Runtime;

use super::{
    FRAME_S, HOLD_PT, Host, SIZE_PT, assert_near, bottom, message, rect, runtime, until_idle,
};
use crate::design::{Density, MessageLayout};
use crate::timeline::anchor::Anchor;
use crate::timeline::rows::anchor_key;
use crate::timeline::{Request, scroll_to};

/// `count` messages from two authors taking turns, a minute apart.
fn long_history(count: u128) -> Vector<Arc<TimelineItem>> {
    (0..count)
        .map(|n| {
            let author = if n % 3 == 0 { "bia" } else { "ana" };
            let body = "words that wrap ".repeat(usize::try_from(n % 9).unwrap() + 1);
            message(n + 1_000, author, u32::try_from(n * 60).unwrap(), &body)
        })
        .collect()
}

/// Scrolls up from the bottom, and returns the first message in view (the
/// one the anchor names) with its top.
fn read_up(rt: &mut Runtime<Host>, wheel_pt: f32) -> (String, f32) {
    for _ in 0..4 {
        rt.wheel(400.0, 300.0, wheel_pt / 4.0);
        rt.frame(FRAME_S);
    }
    until_idle(rt);
    reader_row(rt)
}

fn reader_row(rt: &Runtime<Host>) -> (String, f32) {
    rt.elements()
        .into_iter()
        .filter(|element| {
            let last = element.path.rsplit('/').next().unwrap_or_default();
            last.starts_with("message-") && bottom(element.rect) > 0.0
        })
        .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1]))
        .map(|element| (element.path, element.rect[1]))
        .expect("a message row in view")
}

fn top_of(rt: &Runtime<Host>, path: &str) -> f32 {
    rt.elements()
        .into_iter()
        .find(|element| element.path == path)
        .unwrap_or_else(|| panic!("{path} left the window"))
        .rect[1]
}

#[test]
fn the_message_being_read_keeps_its_place_when_the_density_changes() {
    let mut rt = runtime(long_history(300), MessageLayout::default());
    read_up(&mut rt, 1_500.0);
    let viewport = format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7));
    for density in [
        Density::Compact,
        Density::Comfortable,
        Density::ExtraCompact,
        Density::Normal,
    ] {
        let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
            panic!("reading, not following the newest");
        };
        // What the Appearance settings will do: change it, keep the place.
        rt.update(|host, tx| {
            let anchor = host.open.current_anchor();
            host.layout.density = density;
            scroll_to(anchor, tx);
        });
        until_idle(&mut rt);
        let now_pt = rect(&rt, &anchor_key(id))[1] - rect(&rt, &viewport)[1];
        let moved = now_pt - offset_pt;
        assert!(moved.abs() < HOLD_PT, "{density:?} moved it {moved}");
    }
}

#[test]
fn ten_thousand_messages_page_edit_and_reflow_without_moving_the_reader() {
    let mut rt = runtime(long_history(10_000), MessageLayout::default());
    assert!(
        rt.elements().len() < 2_000,
        "only the window is built: {}",
        rt.elements().len()
    );
    let (path, top) = read_up(&mut rt, 2_000.0);
    let hold = |rt: &Runtime<Host>, step: &str| {
        let moved = top_of(rt, &path) - top;
        assert!(moved.abs() < HOLD_PT, "{step} moved it {moved}");
    };

    // An older page arrives above.
    rt.update(|host, _tx| {
        for n in 0..50 {
            host.open
                .items
                .push_front(message(n + 1, "ana", u32::try_from(n).unwrap(), "older"));
        }
        host.open.keys_version += 1;
    });
    until_idle(&mut rt);
    hold(&rt, "a prepended page");

    // A message above the view is edited and grows.
    let reader = rt
        .state
        .open
        .items
        .iter()
        .position(|item| path.contains(&crate::timeline::row_key(item)))
        .expect("the reader's item");
    rt.update(|host, _tx| {
        let above = reader - 4;
        let TimelineItem::Message {
            id,
            message,
            content,
            ..
        } = host.open.items[above].as_ref()
        else {
            panic!("a message above");
        };
        let mut edited = Message::clone(message);
        edited.content = Some("edited and much longer ".repeat(12));
        edited.edited_at = Some("2026-11-21T09:00:00Z".to_owned());
        let edited = Arc::new(TimelineItem::Message {
            id: *id,
            message: Arc::new(edited),
            content: Arc::clone(content),
            reply: None,
            pending: None,
        });
        host.open.items.set(above, edited);
    });
    until_idle(&mut rt);
    hold(&rt, "an edit above");

    // The window narrows: every row reflows.
    rt.set_viewport((560.0, SIZE_PT.1), 1.0);
    until_idle(&mut rt);
    hold(&rt, "a reflow");
    assert!(rt.elements().len() < 2_000, "still only the window");
}

#[test]
fn the_reader_is_read_back_as_a_message_and_its_offset() {
    let mut rt = runtime(long_history(300), MessageLayout::default());
    assert_eq!(rt.state.open.current_anchor(), Anchor::Newest);
    read_up(&mut rt, 1_500.0);
    let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
        panic!("not following the newest any more");
    };
    let viewport_top = rect(
        &rt,
        &format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7)),
    )[1];
    let row_top = rect(&rt, &anchor_key(id))[1];
    assert_near(offset_pt, row_top - viewport_top, "the anchor's offset");
}

#[test]
fn nearing_the_top_asks_for_the_older_page_once() {
    let mut rt = runtime(long_history(60), MessageLayout::default());
    read_up(&mut rt, 100_000.0);
    let older = |rt: &Runtime<Host>| {
        rt.state
            .requests
            .iter()
            .filter(|request| matches!(request, Request::Older))
            .count()
    };
    assert_eq!(older(&rt), 1);
    read_up(&mut rt, -200.0);
    read_up(&mut rt, 200.0);
    assert_eq!(older(&rt), 1, "one page at a time");
}

#[test]
fn an_idle_timeline_draws_nothing() {
    let mut rt = runtime(long_history(500), MessageLayout::default());
    for _ in 0..120 {
        rt.frame(FRAME_S);
    }
    assert!(!rt.frame(FRAME_S));
    assert!(rt.wake_deadline().is_none());
}
