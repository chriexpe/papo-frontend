//! The shell's side of the timeline: which one the navigation shows, the
//! watches that keep each open one current, the history requests its rows
//! call for, and saving where the reader is.
//!
//! The timelines and their places are `Tabs`'; a timeline's own state is
//! `OpenTimeline`'s. Every post and answer names the timeline and the
//! opening it belongs to, and is dropped when that opening is gone.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use eyeball_im::VectorDiff;
use papo_sdk::{ReadingPosition, Uuid};
use papo_sdk_ui::{Timeline, TimelineDirection, TimelineError, TimelinePage};
use rune::Tx;

use super::Shell;
use crate::tabs::{Opening, Showing};
use crate::timeline::anchor::Anchor;
use crate::timeline::watch::{Posted, Watch};
use crate::timeline::{OpenTimeline, Request, TimelineHost, TimelineKey, scroll_to};

/// How long the reader's place must rest before it is saved, in seconds.
/// Hiding the timeline or closing the window saves it at once.
const SAVE_AFTER_S: f64 = 2.0;

impl Shell {
    /// The timeline the navigation shows: the selected server's account and
    /// its selected text channel.
    fn wanted_timeline(&self) -> Option<TimelineKey> {
        let server = self.nav.selected()?;
        let account = server.account.as_ref()?.id;
        let channel = self.nav.selected_channel()?;
        if !server.text_channels.iter().any(|text| text.id == channel) {
            return None;
        }
        let channel = Uuid::parse_str(channel).ok()?;
        Some(TimelineKey { account, channel })
    }

    /// Shows the timeline the navigation now calls for: hides the one
    /// shown, saving its place, and opens or scrolls back to the next.
    /// Without `tx` (a handler that has none) a timeline shown again opens
    /// at its bottom.
    pub(crate) fn sync_timeline(&mut self, tx: Option<&mut Tx>) {
        if self.sdk.is_none() {
            return;
        }
        let Some(switch) = self.tabs.show(self.wanted_timeline()) else {
            return;
        };
        if let Some(save) = switch.save {
            self.save_position(save.key, save.position);
        }
        match switch.shown {
            Showing::Nothing => {}
            Showing::Open(opening) => self.start_watch(opening),
            Showing::Again(anchor) => {
                if let Some(tx) = tx {
                    scroll_to(anchor, tx);
                }
            }
        }
    }

    /// Closes the timelines of accounts no longer saved.
    pub(crate) fn retain_timelines(&mut self) {
        let accounts: Vec<_> = self
            .nav
            .servers()
            .servers
            .iter()
            .flat_map(|server| server.accounts.iter().map(|account| account.id))
            .collect();
        self.tabs
            .retain_accounts(|account| accounts.contains(&account));
    }

    fn start_watch(&self, opening: Opening) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let Opening {
            key,
            ticket,
            anchor,
            stopped,
        } = opening;
        let watch = Watch {
            key,
            ticket,
            anchor: anchor.and_then(Anchor::message),
            offsets: Arc::clone(&self.local_time.offsets),
            apply: Shell::timeline_posted,
        };
        sdk.watch(move |client, updates| watch.run(client, updates, stopped));
    }

    /// A timeline's watch posted. Returns whether the view changed.
    fn timeline_posted(
        &mut self,
        key: TimelineKey,
        ticket: u64,
        posted: Posted,
        tx: &mut Tx,
    ) -> bool {
        let shown = self.tabs.is_shown(key);
        let Some(open) = self.tabs.get_mut(key, ticket) else {
            log::debug!("dropped a post from a closed timeline");
            return false;
        };
        match posted {
            Posted::Opened(window) => {
                open.opened(window);
                if shown && let Some(anchor) = open.take_restore() {
                    scroll_to(anchor, tx);
                }
            }
            Posted::Update(update) => {
                let reset = update
                    .diffs
                    .iter()
                    .any(|diff| matches!(diff, VectorDiff::Reset { .. }));
                let anchor = open.current_anchor();
                open.update(update);
                // A reset replaces every row; the reader stays where they were.
                if reset && shown {
                    open.restore = Some(anchor);
                    if let Some(anchor) = open.take_restore() {
                        scroll_to(anchor, tx);
                    }
                }
            }
            Posted::Resources(resources) => open.resources = Some(resources),
            Posted::Failed => open.phase = crate::timeline::Phase::Failed,
            Posted::Closed => open.phase = crate::timeline::Phase::Closed,
        }
        shown
    }

    /// Sends the history requests the shown timeline's rows call for.
    fn send_requests(&self, key: TimelineKey, requests: Vec<Request>) {
        let (Some(sdk), Some(open)) = (&self.sdk, self.tabs.shown()) else {
            return;
        };
        let (Some(handle), ticket) = (open.handle().cloned(), open.ticket) else {
            return;
        };
        for request in requests {
            let handle = handle.clone();
            sdk.watch(move |_client, updates| async move {
                let answer = answer(&handle, request).await;
                updates.send(move |shell: &mut Shell| shell.timeline_answered(key, ticket, answer));
            });
        }
    }

    /// A history request came back. Returns whether the view changed.
    fn timeline_answered(&mut self, key: TimelineKey, ticket: u64, answer: Answer) -> bool {
        let Some(open) = self.tabs.get_mut(key, ticket) else {
            return false;
        };
        match answer {
            Answer::Older(page) => {
                open.older_loaded(page.as_ref());
                false
            }
            Answer::Newer(page) => {
                if open.newer_loaded(page.as_ref())
                    && let Some(opening) = self.tabs.reopen(key, ticket)
                {
                    log::debug!("a focused timeline reached the newest; reopening it live");
                    self.start_watch(opening);
                }
                false
            }
            Answer::GapFilled(filled) => {
                open.gap_filled(filled);
                true
            }
        }
    }

    /// Saves where the shown timeline's reader is, when it moved since the
    /// last save.
    pub(crate) fn save_shown_position(&mut self) {
        let Some(open) = self.tabs.shown_mut() else {
            return;
        };
        let key = open.key;
        if let Some(place) = open.place_to_save() {
            self.save_position(key, place.to_saved());
        }
    }

    fn save_position(&self, key: TimelineKey, position: Option<ReadingPosition>) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let Ok(session) = client.session(key.account) else {
                return;
            };
            if let Err(error) = session.save_reading_position(key.channel, position).await {
                log::warn!("a reading position was not saved: {error}");
            }
        });
    }

    /// The window's poll: saves the reader's place once it has rested.
    pub(crate) fn poll(&mut self, clock_s: f64, _tx: &mut Tx) -> bool {
        if self.save_due_s.is_some_and(|due_s| due_s <= clock_s) {
            self.save_due_s = None;
            self.save_shown_position();
        }
        false
    }

    /// The window closes: the shown timeline's place is saved now.
    pub(crate) fn window_closing(&mut self, _tx: &mut Tx) {
        self.save_shown_position();
    }

    /// The reader's place moved: it is saved once it rests.
    fn place_moved(&mut self, tx: &mut Tx) {
        if self.save_due_s.is_none() {
            self.save_due_s = Some(tx.clock() + SAVE_AFTER_S);
            tx.wake_after(Duration::from_secs_f64(SAVE_AFTER_S));
        }
    }

    fn shown_timeline_mut(&mut self, key: TimelineKey) -> Option<&mut OpenTimeline> {
        self.tabs.shown_mut().filter(|open| open.key == key)
    }
}

impl TimelineHost for Shell {
    fn timeline(&self) -> Option<&OpenTimeline> {
        self.tabs.shown()
    }

    fn rows_visible(&mut self, key: TimelineKey, range: Range<usize>) -> bool {
        let Some(open) = self.shown_timeline_mut(key) else {
            return false;
        };
        let requests = open.rows_visible(range);
        let filling = requests
            .iter()
            .any(|request| matches!(request, Request::FillGap(_)));
        self.send_requests(key, requests);
        filling
    }

    fn viewport_moved(&mut self, key: TimelineKey, top_pt: f32, _tx: &mut Tx) {
        if let Some(open) = self.shown_timeline_mut(key) {
            open.tracker.viewport_moved(top_pt);
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
            self.place_moved(tx);
        }
    }

    fn bottom_shown(&mut self, key: TimelineKey, at_bottom: bool) -> bool {
        if let Some(open) = self.shown_timeline_mut(key) {
            open.tracker.bottom_shown(at_bottom);
        }
        false
    }

    fn retry_gap(&mut self, key: TimelineKey, cursor_id: &str) {
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if let Some(gap) = open.retry_gap(cursor_id) {
            self.send_requests(key, vec![Request::FillGap(gap)]);
        }
    }
}

/// A history request's outcome.
enum Answer {
    Older(Option<TimelinePage>),
    Newer(Option<TimelinePage>),
    /// Whether the gap is gone: filled, or no longer in the window.
    GapFilled(bool),
}

async fn answer(handle: &Timeline, request: Request) -> Answer {
    match request {
        Request::Older => Answer::Older(page(handle.paginate(TimelineDirection::Older).await)),
        Request::Newer => Answer::Newer(page(handle.paginate(TimelineDirection::Newer).await)),
        Request::FillGap(gap) => Answer::GapFilled(match handle.fill_gap(&gap).await {
            Ok(_) | Err(TimelineError::StaleSnapshot) => true,
            Err(error) => {
                log::warn!("a gap in a timeline did not fill: {error}");
                false
            }
        }),
    }
}

fn page(result: Result<TimelinePage, TimelineError>) -> Option<TimelinePage> {
    result
        .inspect(|page| {
            log::debug!(
                "a timeline page brought {} messages (more: {})",
                page.message_count,
                page.has_more
            );
        })
        .inspect_err(|error| log::debug!("a timeline page did not load: {error}"))
        .ok()
}
