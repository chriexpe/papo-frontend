//! A timeline's history requests: the page past either end as the reader
//! nears it (a live window pages only older; a focused one newer too, until
//! it reaches the newest and reopens live), and each gap as it comes near,
//! one fill at a time. The shell sends them and brings the answers back.

use std::ops::Range;

use papo_sdk::HistoryGap;
use papo_sdk_ui::{TimelineItem, TimelinePage};

use super::{OpenTimeline, Phase};

/// Rows within this many of the top start loading the page above them.
const PAGE_AHEAD_ROWS: usize = 20;
/// A gap within this many rows of the visible ones fills itself.
const GAP_AHEAD_ROWS: usize = 10;

/// Loading the page past one end of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Paging {
    Idle,
    Loading,
    /// The last page failed; scrolling near that end asks again.
    Failed,
    /// The channel has nothing more that way.
    Exhausted,
}

impl Paging {
    pub(super) fn can_ask(self) -> bool {
        matches!(self, Paging::Idle | Paging::Failed)
    }

    pub(super) fn loaded(page: Option<&TimelinePage>) -> Paging {
        match page {
            Some(page) if !page.has_more => Paging::Exhausted,
            Some(_) => Paging::Idle,
            None => Paging::Failed,
        }
    }
}

/// A history request the timeline asks the shell to send.
pub(crate) enum Request {
    Older,
    /// Only a focused window, which does not follow new messages, pages
    /// newer.
    Newer,
    FillGap(HistoryGap),
}

impl OpenTimeline {
    /// The rows `range` are visible: records them for the anchor and
    /// returns the history requests the reader's place calls for.
    pub(crate) fn rows_visible(&mut self, range: Range<usize>) -> Vec<Request> {
        let visible = self
            .items
            .iter()
            .skip(range.start)
            .take(range.len())
            .filter_map(|item| match item.as_ref() {
                TimelineItem::Message { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        self.tracker.rows_visible(visible);
        let mut requests = Vec::new();
        if self.phase != Phase::Open {
            return requests;
        }
        if range.start < PAGE_AHEAD_ROWS && self.older.can_ask() {
            self.older = Paging::Loading;
            requests.push(Request::Older);
        }
        if range.end + PAGE_AHEAD_ROWS > self.items.len() && self.newer.can_ask() {
            self.newer = Paging::Loading;
            requests.push(Request::Newer);
        }
        if self.filling.is_none() {
            let near = range.start.saturating_sub(GAP_AHEAD_ROWS)..range.end + GAP_AHEAD_ROWS;
            let gap = self
                .items
                .iter()
                .skip(near.start)
                .take(near.len())
                .find_map(|item| match item.as_ref() {
                    TimelineItem::Gap(gap) if !self.failed_gaps.contains(&gap.cursor.id) => {
                        Some(gap.clone())
                    }
                    _ => None,
                });
            if let Some(gap) = gap {
                self.filling = Some(gap.clone());
                requests.push(Request::FillGap(gap));
            }
        }
        requests
    }

    /// The page above the oldest row came back.
    pub(crate) fn older_loaded(&mut self, page: Option<&TimelinePage>) {
        self.older = Paging::loaded(page);
    }

    /// The page below a focused window's newest row came back. Returns
    /// whether the window reached the newest, and must reopen live to
    /// follow new messages.
    pub(crate) fn newer_loaded(&mut self, page: Option<&TimelinePage>) -> bool {
        self.newer = Paging::loaded(page);
        !self.live && self.newer == Paging::Exhausted
    }

    /// The gap being filled came back, filled or not.
    pub(crate) fn gap_filled(&mut self, filled: bool) {
        if let Some(gap) = self.filling.take()
            && !filled
        {
            self.failed_gaps.insert(gap.cursor.id);
        }
    }

    /// Retry on a failed gap. Returns the gap to fill now, unless another
    /// fill is under way: then it fills itself again when near.
    pub(crate) fn retry_gap(&mut self, cursor_id: &str) -> Option<HistoryGap> {
        if !self.failed_gaps.remove(cursor_id) || self.filling.is_some() {
            return None;
        }
        let gap = self.items.iter().find_map(|item| match item.as_ref() {
            TimelineItem::Gap(gap) if gap.cursor.id == cursor_id => Some(gap.clone()),
            _ => None,
        })?;
        self.filling = Some(gap.clone());
        Some(gap)
    }

    /// Whether a gap is being filled.
    pub(crate) fn has_fill(&self) -> bool {
        self.filling.is_some()
    }
}
