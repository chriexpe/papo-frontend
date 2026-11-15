//! Catch-up interval domain logic, point resolution and saved reading position types.
//! Storage durability lives in `catch_up_store`; snapshot indexing and span coalescing live in `catch_up_index`.

use std::collections::BTreeMap;

use papo_api::{HistoryCursor, Uuid};
use serde::{Deserialize, Serialize};

pub(crate) use crate::catch_up_index::{
    AscSuccessors, SnapshotIndex, contiguous_segments, resolve_point_in_snapshot,
};
use crate::{Generation, ReadPoint, catch_up_index::span_contains};

#[cfg(test)]
#[path = "catch_up_span_tests.rs"]
mod span_tests;
#[cfg(test)]
#[path = "catch_up_tests.rs"]
mod tests;

/// Whether a visible-range report continues monotonically from the caller's preceding viewport
/// or represents an initial frame / discontinuous jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VisibilityContinuity {
    /// Scrolled continuously from the previous viewport in the same timeline window.
    Continuous,
    /// Initial frame or discontinuous jump (open at newest, jump to live, hourglass, search).
    Discontinuous,
}

/// Closed inclusive chronological range `[start, end]` observed inside an active catch-up interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSpan {
    /// Oldest boundary in this observed span, inclusive.
    pub start: ReadPoint,
    /// Newest boundary in this observed span, inclusive.
    pub end: ReadPoint,
}

impl ReadSpan {
    pub(crate) fn normalized(a: ReadPoint, b: ReadPoint) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

/// One skipped unread interval `(start_after, end_before)` independent of the channel read point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpInterval {
    /// Exclusive lower bound: messages at or before this point are already caught up.
    pub start_after: Option<ReadPoint>,
    /// Exclusive upper bound: destination-exposed and bottom-retracted messages at or above this point.
    pub end_before: ReadPoint,
    /// Non-overlapping interior spans already observed via discontinuous context views.
    pub covered_spans: Vec<ReadSpan>,
}

/// Durable catch-up state for one `(account, channel)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpState {
    /// Highest chronological point exposed on this channel, used as the lower anchor on future skips.
    pub seen_to: Option<ReadPoint>,
    /// Active skipped catch-up interval, if any unread gap remains open.
    pub interval: Option<CatchUpInterval>,
}

/// Generation-stamped catch-up state for one signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpSnapshot {
    /// Connection epoch that produced or validated this snapshot.
    pub generation: Generation,
    /// Monotonic publication version within this session.
    pub version: u64,
    /// Channels with tracked catch-up state.
    pub channels: BTreeMap<Uuid, CatchUpState>,
}

/// Resolved boundary markers and availability for one channel's active catch-up interval.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CatchUpView {
    /// True when an active catch-up interval exists.
    pub has_catch_up: bool,
    /// Exclusive start point (`interval.start_after`), if present.
    pub start_after: Option<ReadPoint>,
    /// Exclusive end point (`interval.end_before`), if active.
    pub end_before: Option<ReadPoint>,
    /// First unread foreign message ID in the interval once loaded and contiguous after `start_after`.
    /// Matches the visible start marker so hourglass navigation never lands on the already-read anchor.
    pub start_target: Option<Uuid>,
    /// Cursor of `start_target`, when resolved in the snapshot.
    pub start_cursor: Option<HistoryCursor>,
    /// True when an unloaded prefix immediately after `start_after` requires pagination.
    pub needs_history: bool,
}

impl std::fmt::Debug for CatchUpView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatchUpView")
            .field("has_catch_up", &self.has_catch_up)
            .field("start_after", &self.start_after)
            .field("end_before", &self.end_before)
            .field("start_target", &self.start_target)
            .field("has_start_cursor", &self.start_cursor.is_some())
            .field("needs_history", &self.needs_history)
            .finish()
    }
}

/// Durable client-defined reading position for one `(account, channel)`, stored verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPosition {
    /// Anchor message UUID.
    pub message_id: Uuid,
    /// Signed scroll offset relative to `message_id` in client-defined units.
    pub offset_units: i64,
}

/// Generation-stamped saved reading positions for one signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingPositionsSnapshot {
    /// Connection epoch that produced or validated this snapshot.
    pub generation: Generation,
    /// Monotonic publication version within this session.
    pub version: u64,
    /// Channels with a durable saved reading position.
    pub channels: BTreeMap<Uuid, ReadingPosition>,
}

impl CatchUpState {
    /// Observe one non-empty set of visible points in `index` and update `seen_to` + `interval`.
    /// Returns `(changed, new_last_visible)`.
    pub(crate) fn observe_visible(
        &mut self,
        index: &SnapshotIndex<'_>,
        prev_acknowledged_head: Option<ReadPoint>,
        last_visible: Option<(ReadPoint, ReadPoint)>,
        visible: &[ReadPoint],
        continuity: VisibilityContinuity,
    ) -> (bool, Option<(ReadPoint, ReadPoint)>) {
        if visible.is_empty() {
            return (false, last_visible);
        }
        let before = self.clone();
        self.normalize_points(index);
        let mut sorted: Vec<ReadPoint> = visible.iter().map(|&p| index.resolve(p)).collect();
        sorted.sort();
        sorted.dedup();
        let vis_lo = *sorted.first().expect("checked non-empty");
        let vis_hi = *sorted.last().expect("checked non-empty");
        let prev_head = self
            .seen_to
            .into_iter()
            .chain(prev_acknowledged_head.map(|p| index.resolve(p)))
            .max();

        let contiguous_scroll = continuity == VisibilityContinuity::Continuous
            && last_visible.is_some_and(|(prev_lo, prev_hi)| {
                let prev_lo = index.resolve(prev_lo);
                let prev_hi = index.resolve(prev_hi);
                if vis_hi >= prev_hi {
                    index.is_contiguous(Some(prev_hi), vis_lo)
                        || (prev_lo <= vis_hi && prev_hi >= vis_lo)
                } else {
                    index.is_contiguous(Some(vis_hi), prev_lo)
                        || (vis_lo <= prev_hi && vis_hi >= prev_lo)
                }
            });

        // A jump or initial report past `prev_head` opens/replaces an interval iff unread or unknown
        // foreign content actually sits strictly between `prev_head` and `vis_lo`.
        if let Some(head) = prev_head
            && vis_hi > head
            && !contiguous_scroll
            && vis_lo > head
            && !index.no_unseen_foreign(Some(head), vis_lo, &[])
        {
            self.interval = Some(CatchUpInterval {
                start_after: Some(head),
                end_before: vis_lo,
                covered_spans: Vec::new(),
            });
        }

        let mut spans = contiguous_segments(index, &sorted);
        if continuity == VisibilityContinuity::Continuous
            && let Some((prev_lo, prev_hi)) = last_visible
        {
            let prev_lo = index.resolve(prev_lo);
            let prev_hi = index.resolve(prev_hi);
            if vis_lo > prev_hi && index.is_contiguous(Some(prev_hi), vis_lo) {
                spans.push(ReadSpan::normalized(prev_hi, vis_lo));
            } else if vis_hi < prev_lo && index.is_contiguous(Some(vis_hi), prev_lo) {
                spans.push(ReadSpan::normalized(vis_hi, prev_lo));
            }
        }

        if let Some(interval) = &mut self.interval {
            for span in spans {
                interval.apply_span(index, span);
            }
            if interval.is_closed(index) {
                self.interval = None;
            }
        }

        self.seen_to = Some(self.seen_to.map_or(vis_hi, |s| s.max(vis_hi)));
        let changed = *self != before;
        (changed, Some((vis_lo, vis_hi)))
    }

    /// Re-evaluate an active interval against new canonical history, closing if zero unseen foreign rows remain.
    pub(crate) fn reconcile_snapshot(&mut self, index: &SnapshotIndex<'_>) -> bool {
        let before = self.clone();
        self.normalize_points(index);
        if let Some(interval) = &mut self.interval {
            interval.coalesce(index);
            if interval.is_closed(index) {
                self.interval = None;
            }
        }
        *self != before
    }

    /// Delete any active catch-up interval while optionally advancing `seen_to` to `head`.
    pub(crate) fn clear(&mut self, head: Option<ReadPoint>) -> bool {
        let before = self.clone();
        self.interval = None;
        if let Some(head) = head {
            self.seen_to = Some(self.seen_to.map_or(head, |s| s.max(head)));
        }
        *self != before
    }

    /// Resolve display boundaries and hourglass seek readiness against `index`.
    #[must_use]
    pub(crate) fn view(&self, index: &SnapshotIndex<'_>) -> CatchUpView {
        let Some(interval) = &self.interval else {
            return CatchUpView::default();
        };
        let start_after = interval.start_after.map(|p| index.resolve(p));
        let end_before = index.resolve(interval.end_before);
        if start_after.is_some_and(|s| s >= end_before)
            || index.no_unseen_foreign(start_after, end_before, &interval.covered_spans)
        {
            return CatchUpView::default();
        }
        let start_idx = match start_after {
            Some(s) => index.entries.partition_point(|entry| entry.point <= s),
            None => 0,
        };
        let end_idx = index
            .entries
            .partition_point(|entry| entry.point < end_before);
        let first_unread = index.entries[start_idx..end_idx].iter().find(|entry| {
            entry.foreign
                && !interval
                    .covered_spans
                    .iter()
                    .any(|span| span_contains(*span, entry.point))
        });
        let (start_target, start_cursor, needs_history) = match first_unread {
            Some(entry)
                if index.no_unseen_foreign(start_after, entry.point, &interval.covered_spans) =>
            {
                (Some(entry.point.message_id), entry.cursor.clone(), false)
            }
            _ => (None, None, true),
        };
        CatchUpView {
            has_catch_up: true,
            start_after,
            end_before: Some(end_before),
            start_target,
            start_cursor,
            needs_history,
        }
    }

    fn normalize_points(&mut self, index: &SnapshotIndex<'_>) {
        self.seen_to = self.seen_to.map(|p| index.resolve(p));
        if let Some(interval) = &mut self.interval {
            interval.start_after = interval.start_after.map(|p| index.resolve(p));
            interval.end_before = index.resolve(interval.end_before);
            for span in &mut interval.covered_spans {
                span.start = index.resolve(span.start);
                span.end = index.resolve(span.end);
            }
        }
    }
}
