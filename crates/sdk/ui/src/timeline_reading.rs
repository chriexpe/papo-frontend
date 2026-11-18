//! Separate per-window catch-up boundaries and live arrivals projection.
//! Hourglass prefix resolution lives in `timeline_seek`; item diffs live in `timeline`.

use std::{pin::Pin, sync::Arc};

use futures_util::{Stream, stream};
use papo_sdk::{
    CatchUpSnapshot, ChronologicalKey, Generation, HistoryClosedReason, HistorySnapshot,
    HistoryUpdate, Session, Uuid,
};
use tokio::sync::watch;

use super::{
    Timeline, TimelineError,
    timeline_arrivals::{
        TimelineArrivals, compute_arrivals, initial_live_tail, is_live_complete,
        tail_before_live_arrivals, tail_before_point,
    },
};

/// Active catch-up interval boundaries and hourglass readiness for one timeline window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineCatchUp {
    /// Exclusive start anchor (`start_after`) message ID, if known.
    pub start_after_id: Option<Uuid>,
    /// First unread foreign message ID inside the interval once contiguous after `start_after`.
    /// Matches hourglass navigation; never the old read anchor.
    pub start_target_id: Option<Uuid>,
    /// Exclusive end boundary (`end_before`) message ID.
    pub end_before_id: Uuid,
    /// True when an unloaded prefix after `start_after` requires ascending pagination.
    pub needs_history: bool,
    /// True while a `CatchUpSeek` prefix resolution is actively running.
    pub seeking: bool,
}

/// Combined catch-up and arrival projection published separately from `TimelineItem` diffs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineReadingState {
    /// Monotonic publication version for this window's reading overlay.
    pub version: u64,
    /// Connection generation of the underlying history snapshot.
    pub generation: Generation,
    /// Active catch-up boundaries and hourglass state, if a skipped catch-up interval exists.
    pub catch_up: Option<TimelineCatchUp>,
    /// Behind-head live arrivals state.
    pub arrivals: TimelineArrivals,
}

/// Terminal or updated reading overlay event for one timeline window.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum TimelineReadingEvent {
    /// Complete updated catch-up and arrival projection.
    Update(TimelineReadingState),
    /// Channel or session was closed/revoked cleanly; stream ends after this event.
    Closed(HistoryClosedReason),
}

/// Pull-driven stream of reading overlay updates, independent of `TimelineItem` diffs.
pub type TimelineReadingStream = Pin<Box<dyn Stream<Item = TimelineReadingEvent> + Send>>;

/// Current reading overlay snapshot paired with its subsequent update stream.
pub struct TimelineReadingSubscription {
    /// Baseline reading state captured atomically with stream registration.
    pub state: TimelineReadingState,
    /// Strictly subsequent updates and terminal closure.
    pub stream: TimelineReadingStream,
}

#[derive(Clone, PartialEq, Eq)]
struct OverlayKey {
    raw_generation: Generation,
    raw_version: u64,
    catch_generation: Generation,
    catch_revision: Option<u64>,
    window_seen_to: Option<ChronologicalKey>,
    live_baseline_captured: bool,
    seeking: bool,
}

impl OverlayKey {
    fn capture(
        raw: &HistorySnapshot,
        catch_snap: &CatchUpSnapshot,
        channel: Uuid,
        window_seen_to: Option<ChronologicalKey>,
        live_baseline_captured: bool,
        seeking: bool,
    ) -> Self {
        Self {
            raw_generation: raw.generation,
            raw_version: raw.version,
            catch_generation: catch_snap.generation,
            catch_revision: catch_snap.channels.get(&channel).copied(),
            window_seen_to,
            live_baseline_captured,
            seeking,
        }
    }
}

pub(super) struct ReadingOverlay {
    pub history: watch::Receiver<HistoryUpdate>,
    pub catch_up: watch::Receiver<Arc<CatchUpSnapshot>>,
    raw: Arc<HistorySnapshot>,
    window_seen_to: Option<ChronologicalKey>,
    live_baseline_captured: bool,
    pending_rest_fill: Option<ChronologicalKey>,
    seeking_count: usize,
    last_key: OverlayKey,
    pub state: TimelineReadingState,
    pub closed: Option<HistoryClosedReason>,
    pub tx: watch::Sender<TimelineReadingEvent>,
}

impl ReadingOverlay {
    pub fn new(
        session: &Session,
        channel: Uuid,
        raw: Arc<HistorySnapshot>,
        mut history: watch::Receiver<HistoryUpdate>,
        mut catch_up: watch::Receiver<Arc<CatchUpSnapshot>>,
    ) -> Result<Self, TimelineError> {
        let _ = history.borrow_and_update();
        let catch_snap = catch_up.borrow_and_update().clone();
        let live_baseline_captured = is_live_complete(&raw)?;
        let window_seen_to = if live_baseline_captured {
            initial_live_tail(&raw)?
        } else {
            None
        };
        let last_key = OverlayKey::capture(
            &raw,
            &catch_snap,
            channel,
            window_seen_to,
            live_baseline_captured,
            false,
        );
        let initial = project_reading_state(session, channel, &raw, window_seen_to, false, 1)?;
        let tx = watch::channel(TimelineReadingEvent::Update(initial.clone())).0;
        Ok(Self {
            history,
            catch_up,
            raw,
            window_seen_to,
            live_baseline_captured,
            pending_rest_fill: None,
            seeking_count: 0,
            last_key,
            state: initial,
            closed: None,
            tx,
        })
    }

    pub fn refresh(&mut self, session: &Session, channel: Uuid) -> Result<(), TimelineError> {
        self.pull_updates()?;
        let catch_snap = self.catch_up.borrow_and_update().clone();
        self.publish_next(session, channel, &catch_snap)
    }

    pub fn record_exposed(
        &mut self,
        session: &Session,
        channel: Uuid,
        exposed: Option<ChronologicalKey>,
    ) -> Result<(), TimelineError> {
        self.pull_updates()?;
        if let Some(point) = exposed {
            let resolved = self.raw.resolve(point)?;
            if Some(resolved.key) > self.window_seen_to {
                self.window_seen_to = Some(resolved.key);
            }
        }
        let catch_snap = self.catch_up.borrow_and_update().clone();
        self.publish_next(session, channel, &catch_snap)
    }

    fn pull_updates(&mut self) -> Result<(), TimelineError> {
        if let Some(reason) = self.closed {
            return Err(TimelineError::Closed(reason));
        }
        let update = self.history.borrow_and_update().clone();
        let next_raw = match update {
            HistoryUpdate::Reset(snapshot) => snapshot,
            HistoryUpdate::Closed(reason) => {
                self.close(reason);
                return Err(TimelineError::Closed(reason));
            }
            _ => return Err(TimelineError::InvalidCursor),
        };
        self.raw = next_raw;
        if !self.live_baseline_captured && is_live_complete(&self.raw)? {
            self.live_baseline_captured = true;
            let tail = tail_before_live_arrivals(&self.raw)?;
            if tail > self.window_seen_to {
                self.window_seen_to = tail;
            }
            if !self.raw.server_live_complete
                && let Some(cursor) = &self.raw.live_arrival_start
            {
                self.pending_rest_fill = Some(ChronologicalKey::from_cursor(cursor)?);
            }
        } else if let Some(floor) = self.pending_rest_fill {
            let tail = tail_before_point(&self.raw, floor)?;
            if tail > self.window_seen_to {
                self.window_seen_to = tail;
            }
            if self.raw.server_live_complete
                || self.window_seen_to.is_some_and(|seen| seen >= floor)
            {
                self.pending_rest_fill = None;
            }
        }
        Ok(())
    }

    fn publish_next(
        &mut self,
        session: &Session,
        channel: Uuid,
        catch_snap: &CatchUpSnapshot,
    ) -> Result<(), TimelineError> {
        let key = OverlayKey::capture(
            &self.raw,
            catch_snap,
            channel,
            self.window_seen_to,
            self.live_baseline_captured,
            self.seeking_count > 0,
        );
        if key == self.last_key {
            return Ok(());
        }
        let next = project_reading_state(
            session,
            channel,
            &self.raw,
            self.window_seen_to,
            self.seeking_count > 0,
            self.state.version,
        )?;
        self.last_key = key;
        if next.generation != self.state.generation
            || next.catch_up != self.state.catch_up
            || next.arrivals != self.state.arrivals
        {
            let version = self
                .state
                .version
                .checked_add(1)
                .ok_or(TimelineError::VersionExhausted)?;
            let mut next = next;
            next.version = version;
            self.state = next.clone();
            self.tx.send_replace(TimelineReadingEvent::Update(next));
        }
        Ok(())
    }

    pub fn begin_seeking(&mut self, session: &Session, channel: Uuid) {
        self.seeking_count = self.seeking_count.saturating_add(1);
        self.publish_seeking_change(session, channel);
    }

    pub fn end_seeking(&mut self, session: &Session, channel: Uuid) {
        self.seeking_count = self.seeking_count.saturating_sub(1);
        self.publish_seeking_change(session, channel);
    }

    fn publish_seeking_change(&mut self, session: &Session, channel: Uuid) {
        if self.closed.is_some() {
            return;
        }
        let catch_snap = self.catch_up.borrow_and_update().clone();
        let _ = self.publish_next(session, channel, &catch_snap);
    }

    pub fn close(&mut self, reason: HistoryClosedReason) {
        if self.closed.is_none() {
            self.closed = Some(reason);
            self.tx.send_replace(TimelineReadingEvent::Closed(reason));
        }
    }
}

fn project_reading_state(
    session: &Session,
    channel: Uuid,
    raw: &HistorySnapshot,
    window_seen_to: Option<ChronologicalKey>,
    seeking: bool,
    version: u64,
) -> Result<TimelineReadingState, TimelineError> {
    let catch_up = session
        .catch_up_view(channel, raw)?
        .map(|view| TimelineCatchUp {
            start_after_id: view.start_after.map(|p| p.message_id),
            start_target_id: view.start_target,
            end_before_id: view.end_before.message_id,
            needs_history: view.needs_history,
            seeking,
        });
    let arrivals = compute_arrivals(session.account().user(), raw, window_seen_to)?;
    Ok(TimelineReadingState {
        version,
        generation: raw.generation,
        catch_up,
        arrivals,
    })
}

impl Timeline {
    /// Captures the current reading overlay state atomically with an independent update stream.
    /// Catch-up boundary changes and arrivals emit here without producing `TimelineItem` diffs.
    /// # Errors
    /// Returns terminal closure or cursor/version failures.
    pub fn subscribe_reading(&self) -> Result<TimelineReadingSubscription, TimelineError> {
        let mut overlay = self.inner.reading_lock();
        overlay.refresh(&self.inner.session, self.inner.history.channel())?;
        let state = overlay.state.clone();
        let mut events = overlay.tx.subscribe();
        let _ = events.borrow_and_update();
        let history = overlay.history.clone();
        let catch_up = overlay.catch_up.clone();
        let owner = Arc::clone(&self.inner);
        let stream = stream::unfold(
            (owner, events, history, catch_up, false),
            |(owner, mut events, mut history, mut catch_up, terminated)| async move {
                if terminated {
                    return None;
                }
                loop {
                    tokio::select! {
                        biased;
                        changed = events.changed() => {
                            if changed.is_err() {
                                return None;
                            }
                            let event = events.borrow_and_update().clone();
                            let ended = matches!(event, TimelineReadingEvent::Closed(_));
                            return Some((event, (owner, events, history, catch_up, ended)));
                        }
                        res = history.changed() => {
                            if res.is_err() {
                                owner.reading_lock().close(HistoryClosedReason::AccountClosed);
                            } else if let Err(err) = owner.refresh_reading()
                                && !matches!(err, TimelineError::Closed(_))
                            {
                                return None;
                            }
                        }
                        res = catch_up.changed() => {
                            if res.is_err() {
                                owner.reading_lock().close(HistoryClosedReason::AccountClosed);
                            } else if let Err(err) = owner.refresh_reading()
                                && !matches!(err, TimelineError::Closed(_))
                            {
                                return None;
                            }
                        }
                    }
                }
            },
        );
        Ok(TimelineReadingSubscription {
            state,
            stream: Box::pin(stream),
        })
    }

    /// Returns the latest catch-up and arrival projection for this timeline window.
    /// # Errors
    /// Returns closure or invalid cursor errors.
    pub fn reading_state(&self) -> Result<TimelineReadingState, TimelineError> {
        let mut overlay = self.inner.reading_lock();
        overlay.refresh(&self.inner.session, self.inner.history.channel())?;
        Ok(overlay.state.clone())
    }

    /// Deliberately clears this channel's catch-up interval with a `synchronous=FULL` commit.
    /// # Errors
    /// Returns durable storage, closure, or generation errors.
    pub async fn clear_catch_up(&self) -> Result<(), TimelineError> {
        self.inner
            .session
            .clear_catch_up(self.inner.history.channel())
            .await?;
        self.inner.refresh_reading()?;
        Ok(())
    }
}
