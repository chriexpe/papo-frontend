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
use papo_sdk::{AccountId, Generation, HistoryGap, ResourceSnapshot, Uuid};
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
    generation: Option<Generation>,
    /// Changes whenever a row's key may have changed at an index: Rune's
    /// `virtual_rows_version`.
    pub(crate) keys_version: u64,
    /// The account's resources, for author names.
    pub(crate) resources: Option<Arc<ResourceSnapshot>>,
    older: Paging,
    newer: Paging,
    /// The gap being filled; one at a time, as the SDK pages one per
    /// direction.
    filling: Option<HistoryGap>,
    /// Gaps whose fill failed, by cursor: they show Retry.
    pub(crate) failed_gaps: HashSet<String>,
    pub(crate) tracker: Tracker,
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
            generation: None,
            keys_version: 0,
            resources: None,
            older: Paging::Idle,
            newer: Paging::Idle,
            filling: None,
            failed_gaps: HashSet::new(),
            tracker: Tracker::new(),
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
            generation,
            items,
            saved,
        } = window;
        self.handle = Some(handle);
        self.live = live;
        self.version = version;
        self.generation = Some(generation);
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

    /// The timeline shows again: the place to scroll back to, once its
    /// rows are there. While it still opens, the opening restores it.
    pub(crate) fn shown_again(&mut self) -> Option<Anchor> {
        if self.phase != Phase::Open {
            return None;
        }
        self.restore = None;
        Some(self.anchor)
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
        self.generation = Some(update.generation);
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
        self.tracker.anchor().unwrap_or(self.anchor)
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
