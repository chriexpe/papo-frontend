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
pub(crate) mod content;
mod grouping;
pub(crate) mod local_time;
mod message_row;
mod paging;
pub(crate) mod rows;
#[cfg(test)]
mod tests;
mod view;
pub(crate) mod watch;

use std::collections::HashSet;
use std::sync::Arc;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{AccountId, HistoryGap, ResourceSnapshot, Uuid};
use papo_sdk_ui::{Timeline, TimelineItem, TimelineUpdate};
use tokio::sync::oneshot;

use self::anchor::{Anchor, Tracker};
use self::paging::Paging;
pub(crate) use self::paging::Request;
pub(crate) use self::rows::row_key;
pub(crate) use self::view::{Style, TimelineHost, view};
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
    version: u64,
    /// Changes whenever a row's key may have changed at an index: Rune's
    /// `virtual_rows_version`.
    pub(crate) keys_version: u64,
    /// The account's resources, for author and mention names.
    pub(crate) resources: Option<Arc<ResourceSnapshot>>,
    /// Changes whenever `resources` does, so rows naming people redraw.
    pub(crate) resources_version: u64,
    /// The spoilers revealed in each message: they stay revealed while the
    /// timeline is open.
    pub(crate) revealed: imbl::HashMap<Uuid, Vec<u32>>,
    /// The code block whose Copy reads "Copied", until when (seconds on
    /// the window's clock).
    copied: Option<Copied>,
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
            version: 0,
            keys_version: 0,
            resources: None,
            resources_version: 0,
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
        };
        (open, stopped)
    }

    /// An open timeline holding `items`, with no SDK window behind it.
    #[cfg(test)]
    pub(crate) fn loaded(key: TimelineKey, items: Vector<Arc<TimelineItem>>) -> OpenTimeline {
        let (mut open, _stopped) = OpenTimeline::opening(key, 0, None);
        open.items = items;
        open.phase = Phase::Open;
        open
    }

    /// The SDK opened the window. `saved` is the SDK's reading position,
    /// the anchor when the timeline had none of its own.
    pub(crate) fn opened(&mut self, window: Opened) {
        let Opened {
            handle,
            live,
            version,
            items,
            saved,
        } = window;
        self.handle = Some(handle);
        self.live = live;
        self.version = version;
        self.items = items;
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

    /// The same timeline opens a new window, as `ticket`, at the reader's
    /// place: a focused window that reached the newest reopens live. The
    /// rows stay until the new window's arrive. Returns the receiver its
    /// watch stops on; the old watch stops now.
    pub(crate) fn reopen(&mut self, ticket: u64) -> oneshot::Receiver<()> {
        let (stop, stopped) = oneshot::channel();
        drop(std::mem::replace(&mut self.stop, stop));
        self.ticket = ticket;
        // Requests wait for the new window: answers from the old one would
        // carry the new ticket.
        self.handle = None;
        self.anchor = self.current_anchor();
        self.restore = Some(self.anchor);
        self.newer = Paging::Loading;
        stopped
    }

    /// The timeline is no longer shown: its place is kept for when it is
    /// shown again. Returns that place.
    pub(crate) fn hide(&mut self) -> Anchor {
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
        if self.phase == Phase::Open {
            self.restore = Some(self.anchor);
        }
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

    /// Applies one batch of the SDK's diffs, in order.
    pub(crate) fn update(&mut self, update: TimelineUpdate) {
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
        self.version = update.version;
    }

    /// The account's resources changed.
    pub(crate) fn set_resources(&mut self, resources: Arc<ResourceSnapshot>) {
        self.resources = Some(resources);
        self.resources_version += 1;
    }

    /// The spoilers revealed in message `id`.
    pub(crate) fn revealed(&self, id: Uuid) -> &[u32] {
        self.revealed.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Reveals spoiler `index` of message `id`. Returns whether it was
    /// hidden.
    pub(crate) fn reveal(&mut self, id: Uuid, index: u32) -> bool {
        let revealed = self.revealed.entry(id).or_default();
        if revealed.contains(&index) {
            return false;
        }
        revealed.push(index);
        true
    }

    /// The code block of message `id`, by its text's index, whose Copy
    /// reads "Copied".
    pub(crate) fn copied(&self, id: Uuid) -> Option<usize> {
        self.copied
            .filter(|copied| copied.message == id)
            .map(|copied| copied.index)
    }

    /// Code block `index` of message `id` was copied: its Copy reads
    /// "Copied" until `until_s`.
    pub(crate) fn code_copied(&mut self, id: Uuid, index: usize, until_s: f64) {
        self.copied = Some(Copied {
            message: id,
            index,
            until_s,
        });
    }

    /// Ends a "Copied" whose time is up at `now_s`. Returns whether one
    /// ended.
    pub(crate) fn end_copied(&mut self, now_s: f64) -> bool {
        if self.copied.is_some_and(|copied| copied.until_s <= now_s) {
            self.copied = None;
            return true;
        }
        false
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
        let present = self.items.iter().any(|item| {
            matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if *current == id)
        });
        if !present {
            // An anchor the window could not reach back to (deleted, or no
            // longer cached): the window opened at the newest instead.
            self.anchor = Anchor::Newest;
            return None;
        }
        // Until the viewport reports, it is where the scroll puts it.
        self.tracker.bottom_shown(false);
        Some(anchor)
    }
}

/// A code block whose Copy reads "Copied".
#[derive(Clone, Copy, Debug)]
struct Copied {
    message: Uuid,
    /// The block's text's index in its message.
    index: usize,
    until_s: f64,
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

/// Scrolls the shown viewport so `anchor`'s message is where the anchor
/// says: after a switch back, a reset, or a change of density. Following
/// the newest needs none, as a new viewport opens at its bottom.
pub(crate) fn scroll_to(anchor: Anchor, tx: &mut rune::Tx) {
    if let Anchor::Message { id, offset_pt } = anchor {
        tx.scroll_key_to(rows::anchor_key(id), offset_pt);
    }
}
