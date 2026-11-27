//! The shell's side of reading (`docs/design.md` §6, Reading controls): the
//! read reports of the shown timeline's visible rows, sent only while the
//! window shows and no popup covers the rows; ⌛, which pages to the skipped
//! block and shows it in a window of its own; ↓'s jumps; and clearing the
//! block.
//!
//! What a press means is `timeline::reading`'s; what a report holds is
//! `timeline::reports`'. Every answer names its timeline, and its opening or
//! seek, and is dropped once that is gone.

use papo_sdk::{HistoryError, ReadingError, Uuid};
use papo_sdk_ui::{CatchUpSeek, Timeline, TimelineError};
use rune::Tx;
use tokio::sync::oneshot;

use super::Shell;
use crate::timeline::anchor::Anchor;
use crate::timeline::reading::Jump;
use crate::timeline::reports::{Outcome, Report};
use crate::timeline::rows::anchor_key;
use crate::timeline::{Seek, TimelineKey, viewport_key, watch};

/// What ⌛'s paging came back with.
enum Seeked {
    /// The window around the block, and its first unread message.
    Opened(Timeline, Option<Uuid>),
    /// The block closed meanwhile, or the paging failed.
    Ended,
}

/// What floats over the conversation's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Floating {
    /// The header pills, over the top.
    Header,
    /// The composer and the typing line, over the bottom.
    Foot,
}

impl Shell {
    /// The window went off screen (minimised, hidden, covered whole) or came
    /// back (`App::on_visibility`).
    /// Drafts not saved yet are saved as it goes.
    pub(crate) fn window_visibility(&mut self, shown: bool, tx: &mut Tx) {
        self.window_shown = shown;
        if !shown {
            self.flush_composer();
        }
        self.video_window_shown(shown, tx);
        self.sync_visible_channel();
        self.report_reading();
    }

    /// The header or the foot floating over the conversation is now
    /// `height_pt` tall: the rows under it no longer show, so the next read
    /// report leaves them out, or takes them in.
    pub(crate) fn floating_moved(&mut self, edge: Floating, height_pt: f32) {
        let index = match edge {
            Floating::Header => 0,
            Floating::Foot => 1,
        };
        if (self.floating_insets_pt[index] - height_pt).abs() < 0.5 {
            return;
        }
        self.floating_insets_pt[index] = height_pt;
        let insets = self.floating_insets_pt;
        if let Some(open) = self.tabs.shown_mut() {
            open.tracker.insets_moved(insets);
        }
        self.report_reading();
    }

    /// The read report the shown timeline's rows call for, if one may go
    /// now: never while the window is off screen or a popup covers the
    /// rows. Marks it on its way.
    pub(super) fn report_due_now(&mut self) -> Option<(TimelineKey, u64, Report)> {
        if !self.window_shown || self.popup_shown() {
            return None;
        }
        let open = self.tabs.shown_mut()?;
        let report = open.report_due()?;
        Some((open.key, open.ticket, report))
    }

    /// Sends the read report due now, if any.
    pub(super) fn report_reading(&mut self) {
        // Checked first: a report marked on its way must be sent.
        let Some(handle) = self.tabs.shown().and_then(|open| open.handle().cloned()) else {
            return;
        };
        if self.sdk.is_none() {
            return;
        }
        let Some((key, ticket, report)) = self.report_due_now() else {
            return;
        };
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.watch(move |_client, updates| async move {
            let outcome = match handle
                .report_visible(
                    report.version,
                    report.generation,
                    &report.ids,
                    report.continuity,
                )
                .await
            {
                Ok(()) => {
                    log::debug!(
                        "a read report landed: {} rows, {:?}",
                        report.ids.len(),
                        report.continuity
                    );
                    Outcome::Done
                }
                Err(
                    TimelineError::StaleSnapshot
                    | TimelineError::History(HistoryError::StaleSnapshot)
                    | TimelineError::Reading(ReadingError::History(HistoryError::StaleSnapshot)),
                ) => Outcome::Stale,
                Err(error) => {
                    log::warn!("a read report did not land: {error}");
                    Outcome::Failed
                }
            };
            updates.send(move |shell: &mut Shell| {
                shell.report_answered(key, ticket, &report, outcome);
                false
            });
        });
    }

    /// A read report came back; the next may be due.
    fn report_answered(
        &mut self,
        key: TimelineKey,
        ticket: u64,
        report: &Report,
        outcome: Outcome,
    ) {
        if let Some(open) = self.tabs.get_mut(key, ticket) {
            open.reports.answered(report, outcome);
        }
        self.report_reading();
    }

    /// ⌛: pages to the skipped block's first unread message and shows it
    /// in a window of its own. The rows stay until it opens; Cancel stops
    /// it.
    pub(crate) fn hourglass_pressed(&mut self, key: TimelineKey) {
        let next = self.seeks + 1;
        let offsets = std::sync::Arc::clone(&self.local_time.offsets);
        let (Some(sdk), Some(open)) = (
            &self.sdk,
            self.tabs.shown_mut().filter(|open| open.key == key),
        ) else {
            return;
        };
        if open.seek.is_some() {
            return;
        }
        open.reading.hourglass_pressed();
        let Some(handle) = open.handle().cloned() else {
            return;
        };
        let seek = match handle.seek_catch_up(watch::config(offsets)) {
            Ok(seek) => seek,
            Err(error) => {
                log::debug!("the hourglass found no block to open: {error}");
                return;
            }
        };
        let (pending, stopped) = Seek::new(next);
        open.seek = Some(pending);
        self.seeks = next;
        sdk.watch(move |_client, updates| async move {
            let Some(seeked) = finish(seek, stopped).await else {
                return;
            };
            updates.send(move |shell: &mut Shell| shell.seek_finished(key, next, seeked));
        });
    }

    /// Cancel on ⌛'s pill: the paging stops and the rows stay.
    pub(crate) fn seek_cancelled(&mut self, key: TimelineKey) {
        if let Some(open) = self.shown_timeline_mut(key) {
            open.seek = None;
        }
    }

    /// ⌛'s paging came back. Returns whether the view changed.
    fn seek_finished(&mut self, key: TimelineKey, id: u64, seeked: Seeked) -> bool {
        let Some(open) = self.tabs.shown_mut().filter(|open| open.key == key) else {
            return false;
        };
        if open.seek.as_ref().is_none_or(|seek| seek.id != id) {
            return false;
        }
        open.seek = None;
        let offset_pt = open.tracker.jump_offset_pt();
        if let Seeked::Opened(window, target) = seeked {
            let at = target.map_or(Anchor::Newest, |id| Anchor::Message { id, offset_pt });
            if let Some(opening) = self.tabs.adopt(key, window, at) {
                self.start_watch(opening);
            }
        }
        true
    }

    /// ↓: to the first message that arrived unseen, the newest, or (at the
    /// newest) clears the skipped block.
    pub(crate) fn down_pressed(&mut self, key: TimelineKey, tx: &mut Tx) {
        let hourglass = self.conversations.settings().unread_hourglass;
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        let place = open.place(hourglass);
        match open.reading.down_pressed(place) {
            Jump::ClearBlock => self.clear_block(key),
            Jump::Message(id) => self.jump_to(key, id, tx),
            Jump::Latest => self.jump_to_newest(key, tx),
        }
    }

    /// Shows message `id`: in place when the window holds it, else in a
    /// window opened around it.
    pub(super) fn jump_to(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) {
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if open.holds(id) {
            open.reports.jumped();
            // Instant: rows a glide passes were not read.
            tx.scroll_center_in_instant(viewport_key(key), anchor_key(id));
            return;
        }
        let offset_pt = open.tracker.jump_offset_pt();
        let ticket = open.ticket;
        if let Some(opening) =
            self.tabs
                .reopen(key, ticket, Some(Anchor::Message { id, offset_pt }))
        {
            self.start_watch(opening);
        }
    }

    /// Shows the newest message: the bottom of a live window, else a live
    /// window opened at it.
    fn jump_to_newest(&mut self, key: TimelineKey, tx: &mut Tx) {
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if open.follows_newest() {
            open.reports.jumped();
            tx.scroll_to_bottom_instant(viewport_key(key));
            return;
        }
        let ticket = open.ticket;
        if let Some(opening) = self.tabs.reopen(key, ticket, Some(Anchor::Newest)) {
            self.start_watch(opening);
        }
    }

    /// Clears the shown timeline's skipped block; the overlay follows.
    fn clear_block(&self, key: TimelineKey) {
        let (Some(sdk), Some(handle)) = (
            &self.sdk,
            self.tabs
                .shown()
                .filter(|open| open.key == key)
                .and_then(|open| open.handle().cloned()),
        ) else {
            return;
        };
        sdk.run(move |_client| async move {
            if let Err(error) = handle.clear_catch_up().await {
                log::warn!("the skipped block was not cleared: {error}");
            }
        });
    }
}

/// Runs ⌛'s paging until it opens the window around the block, or until
/// `stopped` fires (Cancel, the timeline closed): then `None`, and dropping
/// the seek stops the paging.
async fn finish(seek: CatchUpSeek, mut stopped: oneshot::Receiver<()>) -> Option<Seeked> {
    let finished = tokio::select! {
        biased;
        _ = &mut stopped => return None,
        finished = seek.finish() => finished,
    };
    Some(match finished {
        Ok(window) => {
            let target = window
                .reading_state()
                .ok()
                .and_then(|state| state.catch_up)
                .and_then(|block| block.start_target_id);
            Seeked::Opened(window, target)
        }
        Err(TimelineError::FocusMissing | TimelineError::Cancelled) => Seeked::Ended,
        Err(error) => {
            log::warn!("the hourglass did not reach the block: {error}");
            Seeked::Ended
        }
    })
}
