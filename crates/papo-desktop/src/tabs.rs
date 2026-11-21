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
    /// An open timeline shown again: the scroll back to its place.
    Again(Anchor),
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
        let shown = match self.open.iter().position(|open| open.key == key) {
            Some(index) => {
                let open = self.open.remove(index);
                self.open.push(open);
                self.open
                    .last_mut()
                    .and_then(OpenTimeline::shown_again)
                    .map_or(Showing::Nothing, Showing::Again)
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
            self.keep_place(closed.key, closed.anchor);
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
        assert!(
            matches!(back.shown, Showing::Nothing),
            "still opening: nothing to scroll"
        );
        assert_eq!(tabs.open_count(), 2);
    }

    #[test]
    fn past_the_bound_the_least_recently_shown_closes_and_keeps_its_place() {
        let mut tabs = Tabs::default();
        for n in 0..=OPEN_TIMELINES as u128 {
            tabs.show(Some(key(n)));
        }
        assert_eq!(tabs.open_count(), OPEN_TIMELINES);
        let again = tabs.show(Some(key(0))).expect("shown");
        let Showing::Open(opening) = again.shown else {
            panic!("closed, so it opens again");
        };
        assert_eq!(opening.anchor, Some(Anchor::Newest));
        assert_eq!(tabs.open_count(), OPEN_TIMELINES);
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
        let Showing::Again(anchor) = back.shown else {
            panic!("still open: it scrolls back");
        };
        assert_eq!(
            anchor,
            Anchor::Message {
                id: message,
                offset_pt: -5.0
            }
        );
        let again = tabs.show(Some(key(2))).expect("switched");
        assert!(again.save.is_none(), "nothing moved: nothing to save");
    }
}
