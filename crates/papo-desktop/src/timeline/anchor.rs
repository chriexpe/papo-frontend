//! Where the reader is in one timeline: the anchor, a message plus its
//! row's offset from the viewport's top in points (never a scroll offset),
//! or the newest message while the list follows its bottom.
//!
//! Rune keeps a virtual list's reader in place itself but does not hand
//! that anchor to the app (`docs/rune-patches.md` R-016), so [`Tracker`]
//! reads it from layout reports: each mounted message row's top, the
//! viewport's top, the visible rows and whether the bottom shows. The SDK
//! keeps the durable copy (`ReadingPosition`); this module converts.

use std::collections::HashMap;

use papo_sdk::{ReadingPosition, Uuid};

/// A timeline position that survives reflow: by message, not by pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Anchor {
    /// Following the newest message.
    Newest,
    /// The row of message `id` has its top `offset_pt` points below the
    /// viewport's top; negative when it starts above it.
    Message { id: Uuid, offset_pt: f32 },
}

impl Anchor {
    /// The anchor a saved reading position names; none saved is the newest.
    pub(crate) fn from_saved(saved: Option<ReadingPosition>) -> Anchor {
        match saved {
            None => Anchor::Newest,
            Some(position) => Anchor::Message {
                id: position.message_id,
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a saved offset is a few thousand points at most"
                )]
                offset_pt: position.offset_units as f32,
            },
        }
    }

    /// The reading position to save: `offset_units` are whole points.
    /// Following the newest saves none.
    pub(crate) fn to_saved(self) -> Option<ReadingPosition> {
        match self {
            Anchor::Newest => None,
            Anchor::Message { id, offset_pt } => Some(ReadingPosition {
                message_id: id,
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "an offset within a viewport fits an i64 after rounding"
                )]
                offset_units: offset_pt.round() as i64,
            }),
        }
    }

    /// The message it names, if any.
    pub(crate) fn message(self) -> Option<Uuid> {
        match self {
            Anchor::Newest => None,
            Anchor::Message { id, .. } => Some(id),
        }
    }
}

/// The layout reports of the shown timeline, from which its anchor is read.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    /// The viewport's top, in window points.
    viewport_top_pt: Option<f32>,
    /// The viewport's height, in points.
    viewport_height_pt: f32,
    /// Whether the viewport shows its bottom. A new viewport opens there.
    at_bottom: bool,
    /// The visible message rows, top to bottom, as last reported.
    visible: Vec<Uuid>,
    /// Each reporting message's top and bottom, in window points. Rows
    /// that were not visible at the last report are dropped then: an
    /// unmounted row stops reporting, and its last place would be stale.
    edges_pt: HashMap<Uuid, (f32, f32)>,
}

impl Tracker {
    /// A tracker for a new viewport, which opens at its bottom.
    pub(crate) fn new() -> Tracker {
        Tracker {
            at_bottom: true,
            ..Tracker::default()
        }
    }

    pub(crate) fn viewport_moved(&mut self, top_pt: f32, height_pt: f32) {
        self.viewport_top_pt = Some(top_pt);
        self.viewport_height_pt = height_pt;
    }

    /// Returns whether it changed.
    pub(crate) fn bottom_shown(&mut self, at_bottom: bool) -> bool {
        std::mem::replace(&mut self.at_bottom, at_bottom) != at_bottom
    }

    pub(crate) fn at_bottom(&self) -> bool {
        self.at_bottom
    }

    /// The visible message rows, top to bottom, as last reported.
    pub(crate) fn visible(&self) -> &[Uuid] {
        &self.visible
    }

    /// Where a message a jump lands on puts its top: a third of the way
    /// down the viewport, as Android's jumps do.
    pub(crate) fn jump_offset_pt(&self) -> f32 {
        self.viewport_height_pt / 3.0
    }

    /// Message `id` is laid out from `top_pt` to `bottom_pt`.
    pub(crate) fn row_moved(&mut self, id: Uuid, top_pt: f32, bottom_pt: f32) {
        self.edges_pt.insert(id, (top_pt, bottom_pt));
    }

    /// The message rows now visible, top to bottom.
    pub(crate) fn rows_visible(&mut self, visible: Vec<Uuid>) {
        self.edges_pt.retain(|id, _| visible.contains(id));
        self.visible = visible;
    }

    /// The anchor the reports give: the newest while the bottom of a window
    /// that `follows_newest` shows, else the first message that shows
    /// below the viewport's top. A row whose message is above it, only the
    /// distance below it showing, is not the one being read. `None` before
    /// the rows have reported.
    pub(crate) fn anchor(&self, follows_newest: bool) -> Option<Anchor> {
        if self.at_bottom && follows_newest {
            return Some(Anchor::Newest);
        }
        let viewport_top_pt = self.viewport_top_pt?;
        self.visible.iter().find_map(|id| {
            let (top_pt, bottom_pt) = self.edges_pt.get(id)?;
            (*bottom_pt > viewport_top_pt).then_some(Anchor::Message {
                id: *id,
                offset_pt: top_pt - viewport_top_pt,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{ReadingPosition, Uuid};

    use super::{Anchor, Tracker};

    #[test]
    fn a_new_viewport_follows_the_newest() {
        assert_eq!(Tracker::new().anchor(true), Some(Anchor::Newest));
    }

    #[test]
    fn the_anchor_is_the_first_visible_row_with_a_top() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut tracker = Tracker::new();
        tracker.bottom_shown(false);
        tracker.viewport_moved(100.0, 400.0);
        tracker.rows_visible(vec![a, b]);
        assert_eq!(tracker.anchor(true), None);
        tracker.row_moved(b, 130.0, 130.0 + 20.0);
        assert_eq!(
            tracker.anchor(true),
            Some(Anchor::Message {
                id: b,
                offset_pt: 30.0
            })
        );
        tracker.row_moved(a, 80.0, 120.0);
        assert_eq!(
            tracker.anchor(true),
            Some(Anchor::Message {
                id: a,
                offset_pt: -20.0
            })
        );
    }

    #[test]
    fn the_bottom_of_a_window_that_does_not_follow_is_a_message() {
        let a = Uuid::from_u128(1);
        let mut tracker = Tracker::new();
        tracker.viewport_moved(0.0, 400.0);
        tracker.rows_visible(vec![a]);
        tracker.row_moved(a, 10.0, 30.0);
        assert_eq!(tracker.anchor(true), Some(Anchor::Newest));
        assert_eq!(
            tracker.anchor(false),
            Some(Anchor::Message {
                id: a,
                offset_pt: 10.0
            })
        );
    }

    #[test]
    fn a_row_that_scrolled_out_is_forgotten() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut tracker = Tracker::new();
        tracker.bottom_shown(false);
        tracker.viewport_moved(0.0, 400.0);
        tracker.rows_visible(vec![a, b]);
        tracker.row_moved(a, -10.0, -10.0 + 20.0);
        tracker.row_moved(b, 40.0, 40.0 + 20.0);
        tracker.rows_visible(vec![b]);
        // `a` returns with a stale top until it reports again.
        tracker.rows_visible(vec![a, b]);
        assert_eq!(
            tracker.anchor(true),
            Some(Anchor::Message {
                id: b,
                offset_pt: 40.0
            })
        );
    }

    #[test]
    fn saved_positions_round_trip_in_whole_points() {
        let id = Uuid::from_u128(9);
        let anchor = Anchor::Message {
            id,
            offset_pt: -12.4,
        };
        let saved = anchor.to_saved();
        assert_eq!(
            saved,
            Some(ReadingPosition {
                message_id: id,
                offset_units: -12
            })
        );
        assert_eq!(
            Anchor::from_saved(saved),
            Anchor::Message {
                id,
                offset_pt: -12.0
            }
        );
        assert_eq!(Anchor::Newest.to_saved(), None);
        assert_eq!(Anchor::from_saved(None), Anchor::Newest);
    }
}
