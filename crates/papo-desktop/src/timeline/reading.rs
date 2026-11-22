//! What one timeline shows of the reader's reading (`docs/design.md` §6,
//! Reading controls): the SDK's reading overlay (a skipped block of unread
//! messages, the others' messages that arrived unseen), the new-messages
//! divider held for the visit, and the steps ↓ takes.
//!
//! Pure: the shell carries out a jump and the view draws. The SDK owns the
//! block, the arrivals and the unread marker; this only decides which of
//! them shows where, and what a press on ↓ means now.

use papo_sdk::{MessageKey, Uuid};
use papo_sdk_ui::{TimelineCatchUp, TimelineItem, TimelineReadingState};

use super::OpenTimeline;

/// The reading controls of one open timeline.
#[derive(Debug, Default)]
pub(crate) struct Reading {
    /// The SDK's overlay as last posted; none until the watch posts it.
    overlay: Option<TimelineReadingState>,
    divider: Divider,
    step: Step,
}

/// The new-messages divider of this visit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Divider {
    /// Taken from the next items: the visit's first.
    #[default]
    Unset,
    /// Above this message, or none, until the timeline is left. The SDK's
    /// marker moves the moment rows are reported read; this does not.
    Held(Option<Uuid>),
}

/// Which press of ↓ comes next (Android's `LatestPhase`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Step {
    /// With arrivals, to the first of them.
    #[default]
    FirstNew,
    /// To the newest message.
    Latest,
    /// At the newest message, the next press clears the skipped block.
    ClearBlock,
}

/// What a press on ↓ does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Jump {
    /// Clears the skipped block (`Timeline::clear_catch_up`).
    ClearBlock,
    /// Shows this message: the first that arrived unseen.
    Message(Uuid),
    /// Shows the newest message.
    Latest,
}

/// A line drawn over the gap above a message, with its chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Boundary {
    /// "New messages ↓": the first unread message of the visit.
    NewMessages,
    /// "Unread ↓": the skipped block's first unread message.
    UnreadBelow,
    /// "Unread ↑": the message that ends the skipped block.
    UnreadAbove,
}

/// Where the reader is, as ↓ needs to know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    /// The viewport shows its bottom.
    pub(crate) at_bottom: bool,
    /// The window follows new messages: its bottom is the newest message.
    pub(crate) live: bool,
    /// The "Unread hourglass" setting.
    pub(crate) hourglass: bool,
}

impl Place {
    fn at_newest(self) -> bool {
        self.at_bottom && self.live
    }
}

impl Reading {
    /// The SDK posted its overlay.
    pub(crate) fn overlay_changed(&mut self, overlay: TimelineReadingState) {
        self.overlay = Some(overlay);
    }

    /// The timeline is shown again: a new visit takes its divider from
    /// `items` and starts ↓'s steps over.
    pub(crate) fn visit(&mut self, items: Option<&imbl::Vector<std::sync::Arc<TimelineItem>>>) {
        self.divider = Divider::Unset;
        self.step = Step::FirstNew;
        if let Some(items) = items {
            self.items_changed(items, false);
        }
    }

    /// The items changed. The visit's first items set the divider; later
    /// ones set it only while it has none and the reader is away from the
    /// bottom (messages that arrive at the bottom are read as they come).
    pub(crate) fn items_changed(
        &mut self,
        items: &imbl::Vector<std::sync::Arc<TimelineItem>>,
        at_bottom: bool,
    ) {
        match self.divider {
            Divider::Unset => self.divider = Divider::Held(first_unread(items)),
            Divider::Held(None) if !at_bottom => {
                if let Some(first) = first_unread(items) {
                    self.divider = Divider::Held(Some(first));
                }
            }
            Divider::Held(_) => {}
        }
    }

    /// The reader left the bottom by hand: ↓'s steps start over.
    pub(crate) fn left_bottom(&mut self) {
        if self.step == Step::ClearBlock {
            self.step = Step::FirstNew;
        }
    }

    /// The reader reached the bottom: after "Latest", the next arrivals
    /// are first new again (Android resets `Latest` at the latest).
    pub(crate) fn reached_bottom(&mut self) {
        if self.step == Step::Latest {
            self.step = Step::FirstNew;
        }
    }

    /// ⌛ was pressed: ↓'s steps start over.
    pub(crate) fn hourglass_pressed(&mut self) {
        self.step = Step::FirstNew;
    }

    /// The skipped block, when the SDK holds one.
    pub(crate) fn catch_up(&self) -> Option<&TimelineCatchUp> {
        self.overlay.as_ref()?.catch_up.as_ref()
    }

    /// How many of the others' messages arrived unseen.
    pub(crate) fn arrivals(&self) -> u64 {
        self.overlay
            .as_ref()
            .map_or(0, |overlay| overlay.arrivals.count)
    }

    /// The line above message `id`, if any. Where two meet, the block's
    /// start wins over the divider, and the divider over the block's end.
    pub(crate) fn boundary(&self, id: Uuid, hourglass: bool) -> Option<Boundary> {
        let block = self.catch_up().filter(|_| hourglass);
        if block.is_some_and(|block| block.start_target_id == Some(id)) {
            return Some(Boundary::UnreadBelow);
        }
        if self.divider == Divider::Held(Some(id)) {
            return Some(Boundary::NewMessages);
        }
        if block.is_some_and(|block| block.end_before_id == id) {
            return Some(Boundary::UnreadAbove);
        }
        None
    }

    /// Whether ⌛ shows: the setting is on and a skipped block is left.
    pub(crate) fn hourglass_shown(&self, hourglass: bool) -> bool {
        hourglass && self.catch_up().is_some()
    }

    /// Whether ↓ shows: the reader is away from the newest message, or a
    /// skipped block is left to clear.
    pub(crate) fn down_shown(&self, place: Place) -> bool {
        !place.at_newest() || self.hourglass_shown(place.hourglass)
    }

    /// What the next press on ↓ does.
    pub(crate) fn next_jump(&self, place: Place) -> Jump {
        if self.step == Step::ClearBlock
            && place.at_newest()
            && self.hourglass_shown(place.hourglass)
        {
            return Jump::ClearBlock;
        }
        if self.step == Step::FirstNew
            && let Some(first) = self.first_arrival()
        {
            return Jump::Message(first);
        }
        Jump::Latest
    }

    /// A press on ↓: what it does, and the step after it.
    pub(crate) fn down_pressed(&mut self, place: Place) -> Jump {
        let jump = self.next_jump(place);
        self.step = match jump {
            Jump::ClearBlock => Step::FirstNew,
            Jump::Message(_) => Step::Latest,
            Jump::Latest => Step::ClearBlock,
        };
        jump
    }

    /// The oldest of the others' messages that arrived unseen.
    fn first_arrival(&self) -> Option<Uuid> {
        let overlay = self.overlay.as_ref()?;
        if overlay.arrivals.count == 0 {
            return None;
        }
        let cursor = overlay.arrivals.oldest_unseen.as_ref()?;
        MessageKey::from_cursor(cursor)
            .ok()
            .map(|key| key.message_id)
    }
}

impl OpenTimeline {
    /// Where the reader is, as ↓ and ⌛ need it.
    pub(crate) fn place(&self, hourglass: bool) -> Place {
        Place {
            at_bottom: self.tracker.at_bottom(),
            live: self.live,
            hourglass,
        }
    }
}

/// The message the SDK's unread marker stands before.
fn first_unread(items: &imbl::Vector<std::sync::Arc<TimelineItem>>) -> Option<Uuid> {
    items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::UnreadMarker { before } => Some(*before),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
