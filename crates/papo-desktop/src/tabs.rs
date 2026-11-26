//! Which timelines stay open, and where the reader was in the others: the
//! server tabs. Each (account, channel) keeps its place while it is hidden;
//! a bounded number keep their SDK window and items too, so switching back
//! is instant, and the least recently shown is closed past that. A closed
//! one keeps only its place, in a second bounded list, and reopens from the
//! SDK's cache at it.
//!
//! With "Open at the newest message" on, every timeline shown opens at its
//! newest message instead; places are still kept and saved, for when it is
//! off again.
//!
//! The selected server and each server's channel are `navigation`'s; the
//! timeline itself is `timeline`'s. The durable place is the SDK's reading
//! position, which the shell saves when a timeline is hidden.

use papo_sdk::{AccountId, ReadingPosition};
use papo_sdk_ui::Timeline;
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
    /// A window already open to take instead of opening one: ⌛'s, around
    /// the skipped block.
    pub(crate) window: Option<Timeline>,
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
    /// The timelines shown this run, most recently shown first: the open
    /// ones, then the closed ones whose place is kept.
    pub(crate) fn recently_shown(&self) -> impl Iterator<Item = TimelineKey> + '_ {
        self.open
            .iter()
            .rev()
            .map(|open| open.key)
            .chain(self.places.iter().rev().map(|(key, _)| *key))
    }

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

    /// The accounts with an open timeline, each once.
    pub(crate) fn open_accounts(&self) -> impl Iterator<Item = AccountId> + '_ {
        let mut seen = std::collections::BTreeSet::new();
        self.open
            .iter()
            .map(|open| open.key.account)
            .filter(move |account| seen.insert(*account))
    }

    /// The open timelines of account `account`, shown or not.
    pub(crate) fn of_account_mut(
        &mut self,
        account: AccountId,
    ) -> impl Iterator<Item = &mut OpenTimeline> + '_ {
        self.open
            .iter_mut()
            .filter(move |open| open.key.account == account)
    }

    /// The open timeline `key`, shown or not.
    pub(crate) fn open_mut(&mut self, key: TimelineKey) -> Option<&mut OpenTimeline> {
        self.open.iter_mut().find(|open| open.key == key)
    }

    /// How many timelines are open.
    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Shows timeline `key`, or none, at its place or, `at_newest`, at its
    /// newest message. Unchanged when it already shows.
    pub(crate) fn show(&mut self, key: Option<TimelineKey>, at_newest: bool) -> Option<Switch> {
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
                Showing::Open(self.open_new(key, at_newest))
            }
            // A focused window cannot show the newest: a live one opens.
            Some(mut open) if at_newest && !open.follows_newest() => {
                open.reading.visit(None);
                let ticket = open.ticket;
                self.open.push(open);
                let reopening = self.reopen(key, ticket, Some(Anchor::Newest));
                reopening.map_or(Showing::Nothing, Showing::Open)
            }
            Some(mut open) => {
                if at_newest {
                    open.anchor = Anchor::Newest;
                }
                open.shown_again();
                self.open.push(open);
                Showing::Again
            }
            None => Showing::Open(self.open_new(key, at_newest)),
        };
        Some(Switch { save, shown })
    }

    /// Opens timeline `key` at its kept place, or `at_newest` at its newest
    /// message, closing the least recently shown past the bound.
    fn open_new(&mut self, key: TimelineKey, at_newest: bool) -> Opening {
        let kept = self
            .places
            .iter()
            .position(|(kept, _)| *kept == key)
            .map(|index| self.places.remove(index).1);
        let anchor = if at_newest {
            Some(Anchor::Newest)
        } else {
            kept
        };
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
            window: None,
            stopped,
        }
    }

    /// Opens the same timeline's window again, at `at`, else at the
    /// reader's place (a focused window that reached the newest reopens
    /// live).
    pub(crate) fn reopen(
        &mut self,
        key: TimelineKey,
        ticket: u64,
        at: Option<Anchor>,
    ) -> Option<Opening> {
        let next = self.tickets + 1;
        let open = self.get_mut(key, ticket)?;
        let at = at.unwrap_or_else(|| open.current_anchor());
        let stopped = open.reopen(next, at);
        self.tickets = next;
        Some(Opening {
            key,
            ticket: next,
            anchor: Some(at),
            window: None,
            stopped,
        })
    }

    /// Timeline `key` takes `window`, opened by ⌛ around the skipped
    /// block, and shows message `at` in it.
    pub(crate) fn adopt(
        &mut self,
        key: TimelineKey,
        window: Timeline,
        at: Anchor,
    ) -> Option<Opening> {
        let ticket = self.open.iter().find(|open| open.key == key)?.ticket;
        let mut opening = self.reopen(key, ticket, Some(at))?;
        opening.window = Some(window);
        Some(opening)
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
mod tests;
