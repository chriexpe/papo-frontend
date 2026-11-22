//! The reading controls' decisions: the held divider, the block's edges,
//! when ⌛ and ↓ show, and ↓'s steps.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Generation, MessageKey, Uuid};
use papo_sdk_ui::{TimelineArrivals, TimelineCatchUp, TimelineItem, TimelineReadingState};

use super::{Boundary, Jump, Place, Reading};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn marked(before: u128) -> Vector<Arc<TimelineItem>> {
    Vector::from(vec![Arc::new(TimelineItem::UnreadMarker {
        before: id(before),
    })])
}

fn overlay(arrivals: u64, first: u128, block: Option<(u128, u128)>) -> TimelineReadingState {
    let oldest_unseen = (arrivals > 0).then(|| {
        MessageKey {
            created_at_us: 1,
            message_id: id(first),
        }
        .to_cursor()
        .unwrap()
    });
    TimelineReadingState {
        version: 1,
        generation: Generation::for_test(1),
        catch_up: block.map(|(start, end)| TimelineCatchUp {
            start_after_id: None,
            start_target_id: Some(id(start)),
            end_before_id: id(end),
            needs_history: false,
            seeking: false,
        }),
        arrivals: TimelineArrivals {
            count: arrivals,
            oldest_unseen,
            live_head: None,
        },
    }
}

const AWAY: Place = Place {
    at_bottom: false,
    live: true,
    hourglass: true,
};
const AT_NEWEST: Place = Place {
    at_bottom: true,
    live: true,
    hourglass: true,
};

#[test]
fn the_divider_holds_where_the_visit_began_while_the_marker_moves() {
    let mut reading = Reading::default();
    reading.items_changed(&marked(5), true);
    assert_eq!(reading.boundary(id(5), true), Some(Boundary::NewMessages));
    // Reported read: the SDK's marker moves on, or goes.
    reading.items_changed(&marked(9), true);
    reading.items_changed(&Vector::new(), true);
    assert_eq!(reading.boundary(id(5), true), Some(Boundary::NewMessages));
    assert_eq!(reading.boundary(id(9), true), None);
}

#[test]
fn without_a_divider_one_comes_only_for_messages_arriving_out_of_sight() {
    let mut reading = Reading::default();
    reading.items_changed(&Vector::new(), true);
    // Arriving at the bottom: read as it comes, no divider.
    reading.items_changed(&marked(7), true);
    assert_eq!(reading.boundary(id(7), true), None);
    // Arriving while the reader is scrolled up.
    reading.items_changed(&marked(8), false);
    assert_eq!(reading.boundary(id(8), true), Some(Boundary::NewMessages));
}

#[test]
fn a_new_visit_takes_the_marker_where_it_is_now() {
    let mut reading = Reading::default();
    reading.items_changed(&marked(5), true);
    reading.visit(Some(&Vector::new()));
    assert_eq!(reading.boundary(id(5), true), None);
    reading.visit(Some(&marked(6)));
    assert_eq!(reading.boundary(id(6), true), Some(Boundary::NewMessages));
}

#[test]
fn the_block_edges_show_only_with_the_hourglass_and_its_start_wins() {
    let mut reading = Reading::default();
    reading.items_changed(&marked(3), true);
    reading.overlay_changed(overlay(0, 0, Some((3, 9))));
    assert_eq!(reading.boundary(id(3), true), Some(Boundary::UnreadBelow));
    assert_eq!(reading.boundary(id(9), true), Some(Boundary::UnreadAbove));
    assert_eq!(reading.boundary(id(3), false), Some(Boundary::NewMessages));
    assert_eq!(reading.boundary(id(9), false), None);
}

#[test]
fn hourglass_and_down_show_as_android_shows_them() {
    let mut reading = Reading::default();
    assert!(!reading.hourglass_shown(true));
    assert!(!reading.down_shown(AT_NEWEST));
    assert!(reading.down_shown(AWAY));
    let focused = Place {
        live: false,
        ..AT_NEWEST
    };
    assert!(
        reading.down_shown(focused),
        "a focused window's bottom is not the newest"
    );
    reading.overlay_changed(overlay(0, 0, Some((3, 9))));
    assert!(reading.hourglass_shown(true));
    assert!(!reading.hourglass_shown(false));
    assert!(reading.down_shown(AT_NEWEST), "a block is left to clear");
    let off = Place {
        hourglass: false,
        ..AT_NEWEST
    };
    assert!(!reading.down_shown(off));
}

#[test]
fn down_goes_to_the_first_arrival_then_the_newest_then_clears_the_block() {
    let mut reading = Reading::default();
    reading.overlay_changed(overlay(3, 21, Some((3, 9))));
    assert_eq!(reading.down_pressed(AWAY), Jump::Message(id(21)));
    assert_eq!(reading.down_pressed(AWAY), Jump::Latest);
    assert_eq!(reading.next_jump(AT_NEWEST), Jump::ClearBlock);
    assert_eq!(reading.down_pressed(AT_NEWEST), Jump::ClearBlock);
    assert_eq!(reading.next_jump(AWAY), Jump::Message(id(21)));
}

#[test]
fn leaving_the_bottom_by_hand_starts_the_steps_over() {
    let mut reading = Reading::default();
    reading.overlay_changed(overlay(0, 0, Some((3, 9))));
    assert_eq!(reading.down_pressed(AWAY), Jump::Latest);
    reading.left_bottom();
    assert_eq!(
        reading.next_jump(AT_NEWEST),
        Jump::Latest,
        "no clear without a fresh press to the newest"
    );
}

#[test]
fn reaching_the_newest_after_latest_makes_arrivals_first_new_again() {
    let mut reading = Reading::default();
    reading.overlay_changed(overlay(2, 30, None));
    assert_eq!(reading.down_pressed(AWAY), Jump::Message(id(30)));
    reading.reached_bottom();
    assert_eq!(reading.next_jump(AWAY), Jump::Message(id(30)));
}

#[test]
fn without_the_hourglass_down_never_clears() {
    let mut reading = Reading::default();
    reading.overlay_changed(overlay(0, 0, Some((3, 9))));
    let off = Place {
        hourglass: false,
        ..AWAY
    };
    assert_eq!(reading.down_pressed(off), Jump::Latest);
    let off_at_newest = Place {
        hourglass: false,
        ..AT_NEWEST
    };
    assert_eq!(reading.down_pressed(off_at_newest), Jump::Latest);
}
