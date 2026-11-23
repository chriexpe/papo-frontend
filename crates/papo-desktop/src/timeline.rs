//! One channel's timeline on the UI side: the SDK window's items as its
//! subscription posted them, the history requests it makes as the reader
//! nears an edge or a gap, and where the reader is.
//!
//! The SDK owns the window, its items and their order (`papo_sdk_ui::
//! Timeline`); this keeps the posted items to draw from, applying each batch
//! of diffs in order, and never builds a list of its own. The watch that
//! posts them is `watch`, the history requests are `paging`'s, the rows are
//! drawn by `view` and `message_row`, the anchor is read by `anchor`. Which
//! timelines stay open is `tabs`'.

pub(crate) mod anchor;
mod boundary;
pub(crate) mod change_marks;
pub(crate) mod changes;
pub(crate) mod content;
mod controls;
pub(crate) mod echoes;
mod grouping;
pub(crate) mod hover_bar;
pub(crate) mod local_time;
mod message_row;
mod paging;
mod reactions;
pub(crate) mod reading;
pub(crate) mod replies;
pub(crate) mod reply_line;
pub(crate) mod reports;
mod row_parts;
pub(crate) mod rows;
pub(crate) mod send_mark;
pub(crate) mod standalone;
#[cfg(test)]
mod tests;
mod view;
pub(crate) mod watch;

use std::collections::HashSet;
use std::sync::Arc;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{AccountId, Generation, HistoryGap, Message, ResourceSnapshot, Uuid};
use papo_sdk_ui::{ChannelPins, Timeline, TimelineItem, TimelineUpdate};
use tokio::sync::oneshot;

pub(crate) use self::anchor::scroll_to;
use self::anchor::{Anchor, Tracker};
use self::changes::Changes;
use self::echoes::Echoes;
pub(crate) use self::message_row::{author_name, created_us};
use self::paging::Paging;
pub(crate) use self::paging::Request;
use self::reading::Reading;
pub(crate) use self::reading::Seek;
use self::reports::Reports;
pub(crate) use self::rows::row_key;
pub(crate) use self::view::{Reacting, Style, TimelineHost, view, viewport_key};
pub(crate) use self::watch::Opened;

/// Which timeline: an account's channel. Another account on the same
/// server has its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TimelineKey {
    pub(crate) account: AccountId,
    pub(crate) channel: Uuid,
}

/// Where the SDK window stands.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Waiting for the SDK to open the window.
    Opening,
    Open,
    /// The window could not open (signed out, no access, storage); nothing
    /// is drawn.
    Failed,
    /// The SDK revoked the window (account removed, channel gone).
    Closed,
}

/// One channel's open timeline.
pub(crate) struct OpenTimeline {
    pub(crate) key: TimelineKey,
    /// This opening's identity: posts and answers carrying another are
    /// from a window closed since, and are dropped.
    pub(crate) ticket: u64,
    /// Dropped with the timeline, which ends its watch.
    stop: oneshot::Sender<()>,
    pub(crate) phase: Phase,
    handle: Option<Timeline>,
    /// Whether the window follows new messages; a focused one (opened
    /// around a distant anchor) pages newer and reopens live at the end.
    live: bool,
    /// The SDK window's items, oldest first.
    pub(crate) items: Vector<Arc<TimelineItem>>,
    /// The local echoes among them, drawn as messages.
    pub(crate) echoes: Echoes,
    /// Your edits and deletions in flight on them.
    pub(crate) changes: Changes,
    /// A message a jump just landed on, and until when on the window's
    /// clock it wears `highlight.selected`.
    pub(crate) flash: Option<(Uuid, f64)>,
    version: u64,
    /// The connection generation the items were read in; none while no
    /// window is open (before the first, between a reopen and the next).
    generation: Option<Generation>,
    /// Changes whenever a row's key may have changed at an index: Rune's
    /// `virtual_rows_version`.
    pub(crate) keys_version: u64,
    /// The account's resources, for author and mention names.
    pub(crate) resources: Option<Arc<ResourceSnapshot>>,
    /// The spoilers revealed in each message: they stay revealed while the
    /// timeline is open.
    pub(crate) revealed: imbl::HashMap<Uuid, Vec<u32>>,
    /// The code block whose Copy reads "Copied", until when (seconds on
    /// the window's clock).
    copied: Option<content::kept::Copied>,
    older: Paging,
    newer: Paging,
    /// The gap being filled; one at a time, as the SDK pages one per
    /// direction.
    filling: Option<HistoryGap>,
    /// Gaps whose fill failed, by cursor: they show Retry.
    pub(crate) failed_gaps: HashSet<String>,
    pub(crate) tracker: Tracker,
    /// The rows last reported visible.
    visible_rows: std::ops::Range<usize>,
    /// Where the reader was when the timeline was last hidden, or the seed
    /// it opened with.
    pub(crate) anchor: Anchor,
    /// The anchor to scroll to once the rows can be reached.
    pub(crate) restore: Option<Anchor>,
    /// The reading position the SDK holds for the channel, as last read
    /// or saved; `Newest` when none.
    saved: Anchor,
    /// The skipped block, the arrivals, the divider and ↓'s steps.
    pub(crate) reading: Reading,
    /// The read reports sent from this timeline's rows.
    pub(crate) reports: Reports,
    /// ⌛'s paging toward the skipped block, while it runs.
    pub(crate) seek: Option<Seek>,
    /// The channel's pins, newest pin first; none while unknown.
    pub(crate) pins: Option<Vector<Arc<Message>>>,
    /// Their handle, which fetches them again.
    pub(crate) pins_handle: Option<ChannelPins>,
}

impl OpenTimeline {
    /// A timeline whose window is opening, at `anchor`. Returns the
    /// receiver its watch stops on.
    pub(crate) fn opening(
        key: TimelineKey,
        ticket: u64,
        anchor: Option<Anchor>,
    ) -> (OpenTimeline, oneshot::Receiver<()>) {
        let (stop, stopped) = oneshot::channel();
        let open = OpenTimeline {
            key,
            ticket,
            stop,
            phase: Phase::Opening,
            handle: None,
            live: true,
            items: Vector::new(),
            echoes: Echoes::default(),
            changes: Changes::default(),
            flash: None,
            version: 0,
            generation: None,
            keys_version: 0,
            resources: None,
            revealed: imbl::HashMap::new(),
            copied: None,
            older: Paging::Idle,
            newer: Paging::Idle,
            filling: None,
            failed_gaps: HashSet::new(),
            tracker: Tracker::new(),
            visible_rows: 0..0,
            anchor: anchor.unwrap_or(Anchor::Newest),
            restore: anchor,
            saved: Anchor::Newest,
            reading: Reading::default(),
            reports: Reports::default(),
            seek: None,
            pins: None,
            pins_handle: None,
        };
        (open, stopped)
    }

    /// An open timeline holding `items`, with no SDK window behind it.
    #[cfg(test)]
    pub(crate) fn loaded(key: TimelineKey, items: Vector<Arc<TimelineItem>>) -> OpenTimeline {
        let (mut open, _stopped) = OpenTimeline::opening(key, 0, None);
        open.items = items;
        open.echoes.refresh(&open.items, key.account.user());
        open.changes.refresh(&open.items);
        open.phase = Phase::Open;
        open.generation = Some(Generation::for_test(1));
        open.reading.items_changed(&open.items, true);
        open
    }

    /// The SDK opened the window. `saved` is the SDK's reading position,
    /// the anchor when the timeline had none of its own.
    pub(crate) fn opened(&mut self, window: Opened) {
        let Opened {
            handle,
            live,
            version,
            generation,
            items,
            saved,
        } = window;
        self.handle = Some(handle);
        self.live = live;
        self.version = version;
        self.generation = Some(generation);
        self.items = items;
        self.echoes.refresh(&self.items, self.key.account.user());
        self.changes.refresh(&self.items);
        // Only rows reported from these items count.
        self.reports.jumped();
        self.reading
            .items_changed(&self.items, self.tracker.at_bottom());
        self.keys_version += 1;
        self.phase = Phase::Open;
        self.older = Paging::Idle;
        self.newer = if live {
            Paging::Exhausted
        } else {
            Paging::Idle
        };
        self.filling = None;
        self.failed_gaps.clear();
        self.saved = saved.unwrap_or(Anchor::Newest);
        if self.restore.is_none() {
            self.anchor = self.saved;
            self.restore = saved;
        }
    }

    /// The same timeline opens a new window, as `ticket`, at `at`: the
    /// reader's place when a focused window reached the newest, else where
    /// a jump goes. The rows stay until the new window's arrive. Returns
    /// the receiver its watch stops on; the old watch stops now.
    pub(crate) fn reopen(&mut self, ticket: u64, at: Anchor) -> oneshot::Receiver<()> {
        let (stop, stopped) = oneshot::channel();
        drop(std::mem::replace(&mut self.stop, stop));
        self.ticket = ticket;
        // Requests wait for the new window: answers from the old one would
        // carry the new ticket.
        self.handle = None;
        self.generation = None;
        self.anchor = at;
        self.restore = Some(at);
        self.newer = Paging::Loading;
        self.reports.reopened();
        // A jump while ⌛ pages wins: the seek would undo it.
        self.seek = None;
        stopped
    }

    /// The timeline is no longer shown: its place is kept for when it is
    /// shown again. Returns that place.
    pub(crate) fn hide(&mut self) -> Anchor {
        // Leaving the timeline stops ⌛'s paging.
        self.seek = None;
        self.anchor = self.current_anchor();
        self.tracker = Tracker::new();
        // Until its rows report, the tracker knows nothing of the place.
        self.tracker.bottom_shown(self.anchor == Anchor::Newest);
        self.anchor
    }

    /// The timeline shows again, in a new viewport: its place is restored
    /// by the next `take_restore`. While it still opens, the opening
    /// restores it.
    pub(crate) fn shown_again(&mut self) {
        let open = self.phase == Phase::Open;
        if open {
            self.restore = Some(self.anchor);
        }
        self.reading.visit(open.then_some(&self.items));
        self.reports.jumped();
    }

    /// The place to keep once the timeline closes: where the reader was,
    /// or the place it was asked to open at. None when it never opened
    /// and had no place of its own: the SDK's saved position stands.
    pub(crate) fn kept_place(&self) -> Option<Anchor> {
        match self.phase {
            Phase::Open => Some(self.current_anchor()),
            Phase::Opening | Phase::Failed | Phase::Closed => self.restore,
        }
    }

    /// Whether the window is gone (it failed to open, or the SDK closed
    /// it): showing the timeline again opens a new one.
    pub(crate) fn is_gone(&self) -> bool {
        matches!(self.phase, Phase::Failed | Phase::Closed)
    }

    /// The place to give the SDK as the reading position, when the
    /// reader's place moved since the last one (`Anchor::to_saved`: the
    /// newest clears it).
    pub(crate) fn place_to_save(&mut self) -> Option<Anchor> {
        if self.phase != Phase::Open {
            return None;
        }
        let anchor = self.current_anchor();
        if anchor.to_saved() == self.saved.to_saved() {
            return None;
        }
        self.saved = anchor;
        Some(anchor)
    }

    /// Applies one batch of the SDK's diffs, in order. `watched`: the
    /// window shows and no popup covers the rows, so messages arriving at
    /// the bottom are seen as they come.
    pub(crate) fn update(&mut self, update: TimelineUpdate, watched: bool) {
        if update.version <= self.version {
            return;
        }
        let mut keys_changed = false;
        for diff in update.diffs {
            keys_changed |= changes_keys(&self.items, &diff);
            diff.apply(&mut self.items);
        }
        if keys_changed {
            self.keys_version += 1;
        }
        self.echoes.refresh(&self.items, self.key.account.user());
        self.changes.refresh(&self.items);
        self.version = update.version;
        self.generation = Some(update.generation);
        let sees_bottom = watched && self.tracker.at_bottom();
        self.reading.items_changed(&self.items, sees_bottom);
    }

    pub(crate) fn handle(&self) -> Option<&Timeline> {
        self.handle.as_ref()
    }

    /// The anchor the reader is at now: from the layout reports, else the
    /// one it opened or was restored with.
    pub(crate) fn current_anchor(&self) -> Anchor {
        if self.restore.is_some() {
            return self.anchor;
        }
        self.tracker.anchor(self.live).unwrap_or(self.anchor)
    }

    /// Whether the window follows new messages, so its viewport follows
    /// its bottom. A focused one stays where the reader is as it pages
    /// newer.
    pub(crate) fn follows_newest(&self) -> bool {
        self.live
    }

    /// The anchor to scroll to now, if the rows it names have arrived:
    /// taken, so it happens once. Following the newest needs no scroll, as
    /// a new viewport opens at its bottom.
    pub(crate) fn take_restore(&mut self) -> Option<Anchor> {
        if self.phase != Phase::Open {
            return None;
        }
        let anchor = self.restore.take()?;
        let id = anchor.message()?;
        if !self.holds(id) {
            // An anchor the window could not reach back to (deleted, or no
            // longer cached): the window opened at the newest instead.
            self.anchor = Anchor::Newest;
            return None;
        }
        // Until the viewport reports, it is where the scroll puts it.
        self.tracker.bottom_shown(false);
        self.reports.jumped();
        Some(anchor)
    }

    /// Whether message `id` is among the items.
    pub(crate) fn holds(&self, id: Uuid) -> bool {
        self.items.iter().any(|item| {
            matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if *current == id)
        })
    }
}

/// Whether applying `diff` to `items` can change the key at an index while
/// the count stays: anything but replacing an item by one of the same
/// identity.
fn changes_keys(items: &Vector<Arc<TimelineItem>>, diff: &VectorDiff<Arc<TimelineItem>>) -> bool {
    match diff {
        VectorDiff::Set { index, value } => items
            .get(*index)
            .is_none_or(|old| row_key(old) != row_key(value)),
        _ => true,
    }
}
