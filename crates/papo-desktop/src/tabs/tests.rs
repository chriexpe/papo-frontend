//! The tabs: opening once, the bound and kept places, failed windows,
//! removed accounts, saving on switch, and opening at the newest.

use papo_sdk::{AccountId, ServerAddress, Uuid};

use papo_sdk::ReadingPosition;

use super::{OPEN_TIMELINES, Showing, Tabs};
use crate::timeline::anchor::Anchor;
use crate::timeline::{OpenTimeline, TimelineKey};

impl Tabs {
    fn show_at_place(&mut self, key: Option<TimelineKey>) -> Option<super::Switch> {
        self.show(key, false)
    }
}

fn key(n: u128) -> TimelineKey {
    let server = ServerAddress::parse("https://tabs.example.invalid")
        .expect("a valid address")
        .id();
    TimelineKey {
        account: AccountId::new(server, Uuid::nil()),
        channel: Uuid::from_u128(n),
    }
}

#[test]
fn a_timeline_shown_for_the_first_time_opens_once() {
    let mut tabs = Tabs::default();
    let switch = tabs.show_at_place(Some(key(1))).expect("shown");
    assert!(matches!(switch.shown, Showing::Open(ref opening) if opening.anchor.is_none()));
    assert!(tabs.show_at_place(Some(key(1))).is_none(), "already shown");
    tabs.show_at_place(Some(key(2)));
    let back = tabs.show_at_place(Some(key(1))).expect("shown again");
    assert!(matches!(back.shown, Showing::Again));
    assert!(
        tabs.shown_mut().expect("shown").take_restore().is_none(),
        "still opening: the opening restores it"
    );
    assert_eq!(tabs.open_count(), 2);
}

#[test]
fn past_the_bound_the_least_recently_shown_closes_and_keeps_its_place() {
    let mut tabs = Tabs::default();
    let place = Anchor::Message {
        id: Uuid::from_u128(40),
        offset_pt: 12.0,
    };
    tabs.show_at_place(Some(key(0)));
    let reading = tabs.shown_mut().expect("shown");
    *reading = OpenTimeline::loaded(key(0), imbl::Vector::new());
    reading.anchor = place;
    reading.tracker.bottom_shown(false);
    // Never opened, no place of its own.
    tabs.show_at_place(Some(key(1)));
    for n in 2..=OPEN_TIMELINES as u128 + 1 {
        tabs.show_at_place(Some(key(n)));
    }
    assert_eq!(tabs.open_count(), OPEN_TIMELINES);
    let Showing::Open(opening) = tabs.show_at_place(Some(key(0))).expect("shown").shown else {
        panic!("closed, so it opens again");
    };
    assert_eq!(opening.anchor, Some(place));
    let Showing::Open(opening) = tabs.show_at_place(Some(key(1))).expect("shown").shown else {
        panic!("closed, so it opens again");
    };
    assert_eq!(
        opening.anchor, None,
        "it never had a place: the SDK's saved one stands"
    );
}

#[test]
fn opening_at_the_newest_ignores_the_kept_place_but_keeps_it() {
    let mut tabs = Tabs::default();
    let place = Anchor::Message {
        id: Uuid::from_u128(40),
        offset_pt: 12.0,
    };
    tabs.keep_place(key(1), place);
    let Showing::Open(opening) = tabs.show(Some(key(1)), true).expect("shown").shown else {
        panic!("opens");
    };
    assert_eq!(opening.anchor, Some(Anchor::Newest));
    let shown = tabs.shown_mut().expect("shown");
    *shown = OpenTimeline::loaded(key(1), imbl::Vector::new());
    shown.anchor = place;
    shown.tracker.bottom_shown(false);
    tabs.show(Some(key(2)), true);
    let back = tabs.show(Some(key(1)), true).expect("shown again");
    assert!(matches!(back.shown, Showing::Again));
    let shown = tabs.shown_mut().expect("shown");
    assert_eq!(shown.anchor, Anchor::Newest, "it opens at the newest");
    tabs.show(Some(key(2)), false);
    let back = tabs.show_at_place(Some(key(1))).expect("shown again");
    assert!(matches!(back.shown, Showing::Again));
}

#[test]
fn a_window_that_failed_opens_anew_when_shown() {
    let mut tabs = Tabs::default();
    tabs.show_at_place(Some(key(1)));
    let failed = tabs.shown_mut().expect("shown");
    failed.phase = crate::timeline::Phase::Failed;
    tabs.show_at_place(Some(key(2)));
    let again = tabs.show_at_place(Some(key(1))).expect("shown");
    assert!(matches!(again.shown, Showing::Open(_)));
    assert_eq!(tabs.open_count(), 2);
}

#[test]
fn a_closed_timelines_watch_stops() {
    let mut tabs = Tabs::default();
    let Showing::Open(mut first) = tabs.show_at_place(Some(key(0))).expect("shown").shown else {
        panic!("opens");
    };
    for n in 1..=OPEN_TIMELINES as u128 {
        tabs.show_at_place(Some(key(n)));
    }
    assert!(
        first
            .stopped
            .try_recv()
            .is_err_and(|error| { error == tokio::sync::oneshot::error::TryRecvError::Closed })
    );
}

#[test]
fn a_removed_accounts_timelines_close() {
    let mut tabs = Tabs::default();
    tabs.show_at_place(Some(key(1)));
    tabs.retain_accounts(|_| false);
    assert_eq!(tabs.open_count(), 0);
    assert!(tabs.shown().is_none());
    assert!(
        tabs.show_at_place(Some(key(1))).is_some(),
        "shown again, it reopens"
    );
}

#[test]
fn switching_away_saves_the_place_once_and_switching_back_returns_to_it() {
    let mut tabs = Tabs::default();
    tabs.show_at_place(Some(key(1)));
    let message = Uuid::from_u128(40);
    let reading = tabs.shown_mut().expect("shown");
    *reading = OpenTimeline::loaded(key(1), imbl::Vector::new());
    reading.tracker.bottom_shown(false);
    reading.tracker.viewport_moved(100.0, 400.0, [0.0, 0.0]);
    reading.tracker.rows_visible(vec![message]);
    reading.tracker.row_moved(message, 95.0, 140.0);

    let away = tabs.show_at_place(Some(key(2))).expect("switched");
    let save = away.save.expect("the place moved");
    assert_eq!(save.key, key(1));
    assert_eq!(
        save.position,
        Some(ReadingPosition {
            message_id: message,
            offset_units: -5
        })
    );
    let back = tabs.show_at_place(Some(key(1))).expect("switched back");
    assert!(matches!(back.shown, Showing::Again), "still open");
    assert_eq!(
        tabs.shown().expect("shown").current_anchor(),
        Anchor::Message {
            id: message,
            offset_pt: -5.0
        }
    );
    let again = tabs.show_at_place(Some(key(2))).expect("switched");
    assert!(again.save.is_none(), "nothing moved: nothing to save");
}
