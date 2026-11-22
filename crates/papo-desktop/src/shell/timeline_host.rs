//! The reports the timeline's rows send the shell (`TimelineHost`): rows
//! seen, the viewport and rows moved, presses inside content, gaps to
//! retry, an unsent message's mark. Each names its timeline; one no longer shown changes nothing.

use std::ops::Range;

use papo_sdk::Uuid;
use rune::Tx;

use super::Shell;
use crate::timeline::content::Press;
use crate::timeline::echoes::SendState;
use crate::timeline::{OpenTimeline, Request, TimelineHost, TimelineKey};

impl TimelineHost for Shell {
    fn timeline(&self) -> Option<&OpenTimeline> {
        self.tabs.shown()
    }

    fn rows_visible(&mut self, key: TimelineKey, range: Range<usize>) -> bool {
        let Some(open) = self.shown_timeline_mut(key) else {
            return false;
        };
        let requests = open.rows_visible(range);
        open.reports.rows_reported();
        let filling = requests
            .iter()
            .any(|request| matches!(request, Request::FillGap(_)));
        self.send_requests(key, requests);
        self.report_reading();
        filling
    }

    fn viewport_moved(&mut self, key: TimelineKey, top_pt: f32, height_pt: f32, _tx: &mut Tx) {
        if let Some(open) = self.shown_timeline_mut(key) {
            open.tracker.viewport_moved(top_pt, height_pt);
        }
    }

    fn row_moved(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        [top_pt, bottom_pt]: [f32; 2],
        tx: &mut Tx,
    ) {
        if let Some(open) = self.shown_timeline_mut(key) {
            open.tracker.row_moved(id, top_pt, bottom_pt);
            // A jump that lands on the same rows reports no new range, but
            // its rows move.
            open.reports.rows_reported();
            self.place_moved(tx);
        }
    }

    /// ↓ shows by whether the bottom does, so the view changes with it.
    fn bottom_shown(&mut self, key: TimelineKey, at_bottom: bool) -> bool {
        let Some(open) = self.shown_timeline_mut(key) else {
            return false;
        };
        let changed = open.tracker.bottom_shown(at_bottom);
        if changed {
            if at_bottom {
                open.reading.reached_bottom();
            } else {
                open.reading.left_bottom();
            }
        }
        changed
    }

    fn content_pressed(&mut self, key: TimelineKey, message: Uuid, press: Press, tx: &mut Tx) {
        self.content_pressed_in(key, message, press, tx);
    }

    fn code_copied(&mut self, key: TimelineKey, message: Uuid, index: usize, tx: &mut Tx) {
        self.code_copied_in(key, message, index, tx);
    }

    fn message_hovered(&mut self, key: TimelineKey, message: Uuid, hovered: bool) {
        self.message_hovered_in(key, message, hovered);
    }

    fn hourglass_pressed(&mut self, key: TimelineKey) {
        self.hourglass_pressed(key);
    }

    fn down_pressed(&mut self, key: TimelineKey, tx: &mut Tx) {
        self.down_pressed(key, tx);
    }

    fn seek_cancelled(&mut self, key: TimelineKey) {
        self.seek_cancelled(key);
    }

    fn send_state_pressed(&mut self, key: TimelineKey, local: Uuid, state: SendState) {
        self.open_send_menu(key, local, state);
    }

    fn retry_gap(&mut self, key: TimelineKey, cursor_id: &str) {
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if let Some(gap) = open.retry_gap(cursor_id) {
            self.send_requests(key, vec![Request::FillGap(gap)]);
        }
        // Otherwise another fill runs; this one is due once it answers.
    }
}
