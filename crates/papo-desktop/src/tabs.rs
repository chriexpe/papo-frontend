//! Which timelines stay open, and where the reader was in the others: the
//! server tabs. Each (account, channel) keeps its place while it is hidden;
//! a bounded number keep their SDK window and items too, so switching back
//! is instant, and the least recently shown is closed past that. A closed
//! one keeps only its place, in a second bounded list, and reopens from the
//! SDK's cache at it.
//!
//! The selected server and each server's channel are `navigation`'s; the
//! timeline itself is `timeline`'s. The durable place is the SDK's reading
//! position, which the shell saves when a timeline is hidden.

use papo_sdk::{AccountId, ReadingPosition};
use tokio::sync::oneshot;

use crate::timeline::anchor::Anchor;
use crate::timeline::{OpenTimeline, TimelineKey};

/// How many timelines keep their SDK window and items.
pub(crate) const OPEN_TIMELINES: usize = 6;
/// How many closed timelines keep their place.
const KEPT_PLACES: usize = 256;

/// The open timelines and the places of closed ones.
#[derive(Default)]
pub(crate) struct Tabs {
    /// Least recently shown first; the shown one, if any, is last.
    open: Vec<OpenTimeline>,
    /// Places of timelines closed past [`OPEN_TIMELINES`], least recent
    /// first.
    places: Vec<(TimelineKey, Anchor)>,
    shown: Option<TimelineKey>,
    /// The last ticket issued; each opening takes the next.
    tickets: u64,
}

/// A window to open: the shell starts its watch.
pub(crate) struct Opening {
    pub(crate) key: TimelineKey,
    pub(crate) ticket: u64,
    /// The place to open at; none uses the SDK's saved reading position.
    pub(crate) anchor: Option<Anchor>,
    /// The watch stops when this fires: the timeline was closed.
    pub(crate) stopped: oneshot::Receiver<()>,
}

/// What showing another timeline asks of the shell.
pub(crate) struct Switch {
    /// The hidden timeline's reading position to save, when its place moved
    /// since it was last saved.
    pub(crate) save: Option<Save>,
    pub(crate) shown: Showing,
}

/// A reading position to give the SDK; none clears it (the reader follows
/// the newest).
pub(crate) struct Save {
    pub(crate) key: TimelineKey,
    pub(crate) position: Option<ReadingPosition>,
}

/// The newly shown timeline's need.
pub(crate) enum Showing {
    /// Nothing: no timeline is shown, or it is still opening.
    Nothing,
    /// A window to open.
    Open(Opening),
    /// An open timeline shown again: its place is restored by the next
    /// `OpenTimeline::take_restore`.
    Again,
}

impl Tabs {
    pub(crate) fn shown(&self) -> Option<&OpenTimeline> {
        let key = self.shown?;
        self.open.last().filter(|open| open.key == key)
    }

    pub(crate) fn shown_mut(&mut self) -> Option<&mut OpenTimeline> {
        let key = self.shown?;
        self.open.last_mut().filter(|open| open.key == key)
    }

    pub(crate) fn is_shown(&self, key: TimelineKey) -> bool {
        self.shown == Some(key)
    }

    /// The open timeline `key`, while `ticket` is its current opening.
    pub(crate) fn get_mut(&mut self, key: TimelineKey, ticket: u64) -> Option<&mut OpenTimeline> {
        self.open
            .iter_mut()
            .find(|open| open.key == key && open.ticket == ticket)
    }

    /// How many timelines are open.
    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Shows timeline `key`, or none. Unchanged when it already shows.
    pub(crate) fn show(&mut self, key: Option<TimelineKey>) -> Option<Switch> {
        if self.shown == key {
            return None;
        }
        let save = self.shown_mut().and_then(|open| {
            open.hide();
            let place = open.place_to_save()?;
            Some(Save {
                key: open.key,
                position: place.to_saved(),
            })
        });
        self.shown = key;
        let Some(key) = key else {
            return Some(Switch {
                save,
                shown: Showing::Nothing,
            });
        };
        let kept = self.open.iter().position(|open| open.key == key);
        let shown = match kept.map(|index| self.open.remove(index)) {
            // A window that failed or was closed opens anew, at its place.
            Some(gone) if gone.is_gone() => {
                if let Some(place) = gone.kept_place() {
                    self.keep_place(key, place);
                }
                Showing::Open(self.open_new(key))
            }
            Some(mut open) => {
                open.shown_again();
                self.open.push(open);
                Showing::Again
            }
            None => Showing::Open(self.open_new(key)),
        };
        Some(Switch { save, shown })
    }

    /// Opens timeline `key` at its kept place, closing the least recently
    /// shown past the bound.
    fn open_new(&mut self, key: TimelineKey) -> Opening {
        let anchor = self
            .places
            .iter()
            .position(|(kept, _)| *kept == key)
            .map(|index| self.places.remove(index).1);
        let ticket = self.next_ticket();
        let (open, stopped) = OpenTimeline::opening(key, ticket, anchor);
        self.open.push(open);
        if self.open.len() > OPEN_TIMELINES {
            // Hidden already, so its place is current; dropping it stops its
            // watch.
            let closed = self.open.remove(0);
            if let Some(place) = closed.kept_place() {
                self.keep_place(closed.key, place);
            }
        }
        Opening {
            key,
            ticket,
            anchor,
            stopped,
        }
    }

    /// Opens the same timeline's window again, at the reader's place (a
    /// focused window that reached the newest reopens live).
    pub(crate) fn reopen(&mut self, key: TimelineKey, ticket: u64) -> Option<Opening> {
        let next = self.tickets + 1;
        let open = self.get_mut(key, ticket)?;
        let stopped = open.reopen(next);
        let anchor = Some(open.anchor);
        self.tickets = next;
        Some(Opening {
            key,
            ticket: next,
            anchor,
            stopped,
        })
    }

    /// Closes the timelines of accounts that are gone, and forgets their
    /// places.
    pub(crate) fn retain_accounts(&mut self, kept: impl Fn(AccountId) -> bool) {
        self.open.retain(|open| kept(open.key.account));
        self.places.retain(|(key, _)| kept(key.account));
        if self.shown.is_some_and(|key| !kept(key.account)) {
            self.shown = None;
        }
    }

    fn keep_place(&mut self, key: TimelineKey, anchor: Anchor) {
        self.places.retain(|(kept, _)| *kept != key);
        self.places.push((key, anchor));
        if self.places.len() > KEPT_PLACES {
            self.places.remove(0);
        }
    }

    fn next_ticket(&mut self) -> u64 {
        self.tickets += 1;
        self.tickets
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{AccountId, ServerAddress, Uuid};

    use papo_sdk::ReadingPosition;

    use super::{OPEN_TIMELINES, Showing, Tabs};
    use crate::timeline::anchor::Anchor;
    use crate::timeline::{OpenTimeline, TimelineKey};

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
        let switch = tabs.show(Some(key(1))).expect("shown");
        assert!(matches!(switch.shown, Showing::Open(ref opening) if opening.anchor.is_none()));
        assert!(tabs.show(Some(key(1))).is_none(), "already shown");
        tabs.show(Some(key(2)));
        let back = tabs.show(Some(key(1))).expect("shown again");
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
        tabs.show(Some(key(0)));
        let reading = tabs.shown_mut().expect("shown");
        *reading = OpenTimeline::loaded(key(0), imbl::Vector::new());
        reading.anchor = place;
        reading.tracker.bottom_shown(false);
        // Never opened, no place of its own.
        tabs.show(Some(key(1)));
        for n in 2..=OPEN_TIMELINES as u128 + 1 {
            tabs.show(Some(key(n)));
        }
        assert_eq!(tabs.open_count(), OPEN_TIMELINES);
        let Showing::Open(opening) = tabs.show(Some(key(0))).expect("shown").shown else {
            panic!("closed, so it opens again");
        };
        assert_eq!(opening.anchor, Some(place));
        let Showing::Open(opening) = tabs.show(Some(key(1))).expect("shown").shown else {
            panic!("closed, so it opens again");
        };
        assert_eq!(
            opening.anchor, None,
            "it never had a place: the SDK's saved one stands"
        );
    }

    #[test]
    fn a_window_that_failed_opens_anew_when_shown() {
        let mut tabs = Tabs::default();
        tabs.show(Some(key(1)));
        let failed = tabs.shown_mut().expect("shown");
        failed.phase = crate::timeline::Phase::Failed;
        tabs.show(Some(key(2)));
        let again = tabs.show(Some(key(1))).expect("shown");
        assert!(matches!(again.shown, Showing::Open(_)));
        assert_eq!(tabs.open_count(), 2);
    }

    #[test]
    fn a_closed_timelines_watch_stops() {
        let mut tabs = Tabs::default();
        let Showing::Open(mut first) = tabs.show(Some(key(0))).expect("shown").shown else {
            panic!("opens");
        };
        for n in 1..=OPEN_TIMELINES as u128 {
            tabs.show(Some(key(n)));
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
        tabs.show(Some(key(1)));
        tabs.retain_accounts(|_| false);
        assert_eq!(tabs.open_count(), 0);
        assert!(tabs.shown().is_none());
        assert!(tabs.show(Some(key(1))).is_some(), "shown again, it reopens");
    }

    #[test]
    fn switching_away_saves_the_place_once_and_switching_back_returns_to_it() {
        let mut tabs = Tabs::default();
        tabs.show(Some(key(1)));
        let message = Uuid::from_u128(40);
        let reading = tabs.shown_mut().expect("shown");
        *reading = OpenTimeline::loaded(key(1), imbl::Vector::new());
        reading.tracker.bottom_shown(false);
        reading.tracker.viewport_moved(100.0);
        reading.tracker.rows_visible(vec![message]);
        reading.tracker.row_moved(message, 95.0, 140.0);

        let away = tabs.show(Some(key(2))).expect("switched");
        let save = away.save.expect("the place moved");
        assert_eq!(save.key, key(1));
        assert_eq!(
            save.position,
            Some(ReadingPosition {
                message_id: message,
                offset_units: -5
            })
        );
        let back = tabs.show(Some(key(1))).expect("switched back");
        assert!(matches!(back.shown, Showing::Again), "still open");
        assert_eq!(
            tabs.shown().expect("shown").current_anchor(),
            Anchor::Message {
                id: message,
                offset_pt: -5.0
            }
        );
        let again = tabs.show(Some(key(2))).expect("switched");
        assert!(again.save.is_none(), "nothing moved: nothing to save");
    }
}
