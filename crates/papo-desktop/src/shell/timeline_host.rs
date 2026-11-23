//! The reports the timeline's rows send the shell (`TimelineHost`): rows
//! seen, the viewport and rows moved, presses inside content, gaps to
//! retry, an unsent message's mark. Each names its timeline; one no longer shown changes nothing.

use std::ops::Range;

use papo_sdk::Uuid;
use rune::{KeyEvent, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::menu::Placement;
use crate::timeline::content::Press;
use crate::timeline::echoes::SendState;
use crate::timeline::hover_bar::{BarPress, more_key};
use crate::timeline::reactors::Want;
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
        // A mention in one of the open pins panel's pins is a press inside
        // the panel: it stays, and the card opens beside the mention.
        let in_pins = matches!(press, Press::Person(_))
            && self.shown_pins_panel().is_some()
            && self
                .shown_timeline(key)
                .is_some_and(|open| open.pins.get(message).is_some());
        if !in_pins {
            self.worked_outside();
        }
        self.content_pressed_in(key, message, press, tx);
    }

    fn code_copied(&mut self, key: TimelineKey, message: Uuid, index: usize, tx: &mut Tx) {
        self.worked_outside();
        self.code_copied_in(key, message, index, tx);
    }

    fn author_pressed(&mut self, key: TimelineKey, user: Uuid, anchor: String, tx: &mut Tx) {
        // A press moves no keyboard focus: what had it gets it back, as
        // from a menu the pointer opened.
        self.worked_outside();
        let trigger = tx.focused().unwrap_or(COMPOSER_KEY).to_owned();
        self.open_card(key, user, Placement::Beside(anchor), trigger, tx);
    }

    fn message_pressed(&mut self, key: TimelineKey) {
        if self.tabs.is_shown(key) {
            self.worked_outside();
        }
    }

    fn timeline_wheeled(&mut self, key: TimelineKey) -> bool {
        self.tabs.is_shown(key) && self.worked_outside()
    }

    fn message_hovered(&mut self, key: TimelineKey, message: Uuid, hovered: bool) -> bool {
        self.message_hovered_in(key, message, hovered);
        hovered && self.fold_strip_for(key, message)
    }

    fn hourglass_pressed(&mut self, key: TimelineKey) {
        self.worked_outside();
        self.hourglass_pressed(key);
    }

    fn down_pressed(&mut self, key: TimelineKey, tx: &mut Tx) {
        self.worked_outside();
        self.down_pressed(key, tx);
    }

    fn seek_cancelled(&mut self, key: TimelineKey) {
        self.seek_cancelled(key);
    }

    fn send_state_pressed(&mut self, key: TimelineKey, local: Uuid, state: SendState) {
        self.worked_outside();
        self.open_send_menu(key, local, state);
    }

    fn message_key(&mut self, key: TimelineKey, id: Uuid, event: &KeyEvent, tx: &mut Tx) -> bool {
        Shell::message_key(self, key, id, event, tx)
    }

    fn reply_line_pressed(&mut self, key: TimelineKey, original: Uuid, tx: &mut Tx) {
        self.worked_outside();
        self.jump_to_original(key, original, tx);
    }

    fn change_mark_pressed(&mut self, key: TimelineKey, local: Uuid) {
        self.worked_outside();
        self.open_change_menu(key, local);
    }

    fn change_retried(&mut self, key: TimelineKey, local: Uuid) {
        self.retry_change(key, local);
    }

    fn change_discarded(&mut self, key: TimelineKey, local: Uuid) {
        self.discard_change(key, local);
    }

    fn bar_pressed(&mut self, key: TimelineKey, id: Uuid, press: BarPress, tx: &mut Tx) {
        self.worked_outside();
        if self.tabs.is_shown(key) {
            self.bar_press(key, id, press, tx);
        }
    }

    fn chip_hovered(&mut self, key: TimelineKey, id: Uuid) {
        if self.tabs.is_shown(key) {
            self.load_reactors(key, id, Want::Hint);
        }
    }

    fn chip_menu_requested(&mut self, key: TimelineKey, id: Uuid, emoji: String, _tx: &mut Tx) {
        self.worked_outside();
        if self.tabs.is_shown(key) {
            self.open_chip_reactions(key, id, &emoji);
        }
    }

    fn link_menu_requested(&mut self, key: TimelineKey, url: String, at: [f32; 2], _tx: &mut Tx) {
        self.worked_outside();
        if self.tabs.is_shown(key) {
            self.open_link_menu(key, url, at);
        }
    }

    fn message_menu_requested(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        at: Option<[f32; 2]>,
        tx: &mut Tx,
    ) {
        self.worked_outside();
        let placement = match at {
            Some(point) => Placement::AtPointer(point),
            None => Placement::BelowEnd(more_key(id)),
        };
        let trigger = super::hover_bar::pointer_trigger(tx);
        self.open_message_menu(key, id, placement, trigger);
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
