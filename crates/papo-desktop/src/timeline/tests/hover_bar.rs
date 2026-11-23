//! The hover bar, headless: unseen and untouchable at rest, seen on the
//! row under the pointer or with the keyboard, on the right row after a
//! scroll under a still pointer (R-019), and nothing drawn once it rests.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::Uuid;
use papo_sdk_ui::TimelineItem;
use rune::Runtime;

use super::{FRAME_S, Host, message, runtime, until_idle};
use crate::design::MessageLayout;
use crate::timeline::rows::message_key;

/// `count` messages, each by another author than the one before.
fn turns(count: u128) -> Vector<Arc<TimelineItem>> {
    (1..=count)
        .map(|n| {
            let author = if n % 2 == 0 { "ana" } else { "bia" };
            message(n, author, u32::try_from(n).unwrap() * 60, "a line of words")
        })
        .collect()
}

/// The opacity message `n`'s bar draws at.
fn bar_opacity(rt: &Runtime<Host>, n: u128) -> Option<f32> {
    let row = format!("/{}/", message_key(Uuid::from_u128(n)));
    rt.elements()
        .into_iter()
        .find(|element| element.path.contains(&row) && element.path.ends_with("/bar"))
        .map(|element| element.opacity)
}

/// The message whose row is under the point.
fn message_at(rt: &Runtime<Host>, x: f32, y: f32) -> Option<u128> {
    let picked = rt.element_at(x, y)?;
    crate::timeline::content::copy::message_of(picked.path()).map(|id| id.as_u128())
}

/// Frames until the bars' fades are done.
fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    until_idle(rt);
}

#[test]
fn the_bar_shows_on_the_row_under_the_pointer_and_only_there() {
    let mut rt = runtime(turns(12), MessageLayout::default());
    let (x, y) = (400.0, 560.0);
    let under = message_at(&rt, x, y).expect("a message under the pointer");
    assert_eq!(bar_opacity(&rt, under), Some(0.0), "unseen at rest");
    rt.pointer_move(x, y);
    settle(&mut rt);
    assert_eq!(bar_opacity(&rt, under), Some(1.0));
    let other = if under > 1 { under - 1 } else { under + 1 };
    assert_eq!(bar_opacity(&rt, other), Some(0.0));
    assert!(!rt.frame(FRAME_S), "a shown bar draws nothing more");
}

#[test]
fn an_unseen_bar_takes_no_press_from_the_row_above() {
    let rt = runtime(turns(12), MessageLayout::default());
    let bars: Vec<[f32; 4]> = rt
        .elements()
        .into_iter()
        .filter(|element| element.path.ends_with("/bar") && element.opacity == 0.0)
        .map(|element| element.rect)
        .collect();
    let bar = bars.last().expect("a bar");
    let (x, y) = (bar[0] + bar[2] / 2.0, bar[1] + 2.0);
    let picked = rt.element_at(x, y).map(|picked| picked.path().to_owned());
    assert!(
        picked.is_none_or(|path| !path.contains("/bar")),
        "an unseen bar is not picked"
    );
}

#[test]
fn the_keyboard_on_a_row_shows_its_bar() {
    let mut rt = runtime(turns(6), MessageLayout::default());
    rt.update(|_host, tx| tx.focus(message_key(Uuid::from_u128(6)), true));
    settle(&mut rt);
    assert_eq!(bar_opacity(&rt, 6), Some(1.0));
    assert_eq!(bar_opacity(&rt, 5), Some(0.0));
}

#[test]
fn after_a_scroll_under_a_still_pointer_the_bar_is_on_the_row_under_it() {
    let mut rt = runtime(turns(200), MessageLayout::default());
    let (x, y) = (400.0, 300.0);
    rt.pointer_move(x, y);
    settle(&mut rt);
    let before = message_at(&rt, x, y).expect("a message under the pointer");
    assert_eq!(bar_opacity(&rt, before), Some(1.0));
    for _ in 0..4 {
        rt.wheel(x, y, 150.0);
        rt.frame(FRAME_S);
    }
    settle(&mut rt);
    let after = message_at(&rt, x, y).expect("a message under the pointer");
    assert_ne!(
        before, after,
        "the scroll moved other rows under the pointer"
    );
    assert_eq!(
        bar_opacity(&rt, after),
        Some(1.0),
        "lit where the pointer is"
    );
    assert_ne!(bar_opacity(&rt, before), Some(1.0), "and nowhere else");
}

#[test]
fn moving_to_the_next_row_never_shows_two_bars() {
    let mut rt = runtime(turns(12), MessageLayout::default());
    let x = 400.0;
    rt.pointer_move(x, 560.0);
    settle(&mut rt);
    let first = message_at(&rt, x, 560.0).expect("a message under the pointer");
    let above = (300_u16..560)
        .rev()
        .map(f32::from)
        .find(|y| message_at(&rt, x, *y).is_some_and(|n| n != first))
        .expect("a row above");
    let second = message_at(&rt, x, above).unwrap();
    rt.pointer_move(x, above);
    rt.frame(FRAME_S);
    assert_eq!(
        bar_opacity(&rt, first),
        Some(0.0),
        "the old bar goes at once"
    );
    settle(&mut rt);
    assert_eq!(bar_opacity(&rt, second), Some(1.0));
}

#[test]
fn the_bars_half_over_the_row_above_keeps_its_own_row() {
    let mut rt = runtime(turns(12), MessageLayout::default());
    let x = 400.0;
    rt.pointer_move(x, 560.0);
    settle(&mut rt);
    let row = message_at(&rt, x, 560.0).expect("a message under the pointer");
    let path = format!("/{}/", message_key(Uuid::from_u128(row)));
    let bar = rt
        .elements()
        .into_iter()
        .find(|element| element.path.contains(&path) && element.path.ends_with("/bar"))
        .expect("its bar")
        .rect;
    // A point on the bar above the row's top edge: over the row above.
    let (bx, by) = (bar[0] + bar[2] / 2.0, bar[1] + 3.0);
    rt.pointer_move(bx, by);
    settle(&mut rt);
    assert_eq!(bar_opacity(&rt, row), Some(1.0), "the bar stays on its row");
    assert_eq!(
        bar_opacity(&rt, row - 1),
        Some(0.0),
        "and the row above shows none"
    );
}
