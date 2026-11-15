//! Separate per-window catch-up boundaries and live arrivals projection.
//! Hourglass prefix resolution lives in `timeline_seek`; item diffs live in `timeline`.

use std::{pin::Pin, sync::Arc};

use futures_util::{Stream, stream};
use papo_sdk::{
    CatchUpSnapshot, CatchUpState, Generation, HistoryClosedReason, HistorySnapshot, HistoryUpdate,
    ReadPoint, ReadState, ReadingSnapshot, Session, Uuid,
};
use tokio::sync::watch;

use super::{
    Timeline, TimelineError,
    timeline_arrivals::{TimelineArrivals, compute_arrivals},
};

/// Catch-up interval boundaries and hourglass readiness for one timeline window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimelineCatchUp {
    /// True when an active skipped catch-up interval exists on this channel.
    pub available: bool,
    /// Exclusive start anchor (`start_after`) message ID, if known.
    pub start_after_id: Option<Uuid>,
    /// First unread foreign message ID inside the interval once contiguous after `start_after`.
    /// Matches hourglass navigation; never the old read anchor.
    pub start_target_id: Option<Uuid>,
    /// Exclusive end boundary (`end_before`) message ID, when active.
    pub end_before_id: Option<Uuid>,
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
    /// Current catch-up boundaries and hourglass availability.
    pub catch_up: TimelineCatchUp,
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
    read_generation: Generation,
    read_state: Option<ReadState>,
    catch_generation: Generation,
    catch_state: Option<CatchUpState>,
    seeking: bool,
}

impl OverlayKey {
    fn capture(
        raw: &HistorySnapshot,
        read_snap: &ReadingSnapshot,
        catch_snap: &CatchUpSnapshot,
        channel: Uuid,
        seeking: bool,
    ) -> Self {
        Self {
            raw_generation: raw.generation,
            raw_version: raw.version,
            read_generation: read_snap.generation,
            read_state: read_snap.channels.get(&channel).cloned(),
            catch_generation: catch_snap.generation,
            catch_state: catch_snap.channels.get(&channel).cloned(),
            seeking,
        }
    }
}

pub(super) struct ReadingOverlay {
    pub history: watch::Receiver<HistoryUpdate>,
    pub reading: watch::Receiver<Arc<ReadingSnapshot>>,
    pub catch_up: watch::Receiver<Arc<CatchUpSnapshot>>,
    raw: Arc<HistorySnapshot>,
    initial_tail: Option<ReadPoint>,
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
        mut reading: watch::Receiver<Arc<ReadingSnapshot>>,
        mut catch_up: watch::Receiver<Arc<CatchUpSnapshot>>,
        initial_tail: Option<ReadPoint>,
    ) -> Result<Self, TimelineError> {
        let _ = history.borrow_and_update();
        let read_snap = reading.borrow_and_update().clone();
        let catch_snap = catch_up.borrow_and_update().clone();
        let last_key = OverlayKey::capture(&raw, &read_snap, &catch_snap, channel, false);
        let initial = project_reading_state(
            session,
            channel,
            &raw,
            read_snap.channels.get(&channel),
            catch_snap.channels.get(&channel),
            initial_tail,
            false,
            1,
        )?;
        let tx = watch::channel(TimelineReadingEvent::Update(initial.clone())).0;
        Ok(Self {
            history,
            reading,
            catch_up,
            raw,
            initial_tail,
            seeking_count: 0,
            last_key,
            state: initial,
            closed: None,
            tx,
        })
    }

    pub fn refresh(&mut self, session: &Session, channel: Uuid) -> Result<(), TimelineError> {
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
        let read_snap = self.reading.borrow_and_update().clone();
        let catch_snap = self.catch_up.borrow_and_update().clone();
        self.raw = next_raw;
        self.publish_next(session, channel, &read_snap, &catch_snap)
    }

    fn publish_next(
        &mut self,
        session: &Session,
        channel: Uuid,
        read_snap: &ReadingSnapshot,
        catch_snap: &CatchUpSnapshot,
    ) -> Result<(), TimelineError> {
        let key = OverlayKey::capture(
            &self.raw,
            read_snap,
            catch_snap,
            channel,
            self.seeking_count > 0,
        );
        if key == self.last_key {
            return Ok(());
        }
        let next = project_reading_state(
            session,
            channel,
            &self.raw,
            read_snap.channels.get(&channel),
            catch_snap.channels.get(&channel),
            self.initial_tail,
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

    pub fn set_seeking(&mut self, session: &Session, channel: Uuid, delta: isize) {
        if delta >= 0 {
            self.seeking_count = self.seeking_count.saturating_add(delta.cast_unsigned());
        } else {
            self.seeking_count = self.seeking_count.saturating_sub(delta.unsigned_abs());
        }
        if self.closed.is_some() {
            return;
        }
        let read_snap = self.reading.borrow_and_update().clone();
        let catch_snap = self.catch_up.borrow_and_update().clone();
        let _ = self.publish_next(session, channel, &read_snap, &catch_snap);
    }

    pub fn close(&mut self, reason: HistoryClosedReason) {
        if self.closed.is_none() {
            self.closed = Some(reason);
            self.tx.send_replace(TimelineReadingEvent::Closed(reason));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn project_reading_state(
    session: &Session,
    channel: Uuid,
    raw: &HistorySnapshot,
    read_state: Option<&ReadState>,
    catch_state: Option<&CatchUpState>,
    initial_tail: Option<ReadPoint>,
    seeking: bool,
    version: u64,
) -> Result<TimelineReadingState, TimelineError> {
    let view = session.catch_up_view(channel, raw)?;
    let catch_up = TimelineCatchUp {
        available: view.has_catch_up,
        start_after_id: view.start_after.map(|p| p.message_id),
        start_target_id: view.start_target,
        end_before_id: view.end_before.map(|p| p.message_id),
        needs_history: view.needs_history,
        seeking,
    };
    let arrivals = compute_arrivals(
        session.account().user(),
        raw,
        read_state,
        catch_state,
        initial_tail,
    )?;
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
        let reading = overlay.reading.clone();
        let catch_up = overlay.catch_up.clone();
        let owner = Arc::clone(&self.inner);
        let stream = stream::unfold(
            (owner, events, history, reading, catch_up, false),
            |(owner, mut events, mut history, mut reading, mut catch_up, terminated)| async move {
                if terminated {
                    return None;
                }
                loop {
                    tokio::select! {
                        changed = events.changed() => {
                            if changed.is_err() {
                                return None;
                            }
                            let event = events.borrow_and_update().clone();
                            let ended = matches!(event, TimelineReadingEvent::Closed(_));
                            return Some((event, (owner, events, history, reading, catch_up, ended)));
                        }
                        res = history.changed() => {
                            if res.is_err() {
                                owner.reading_lock().close(HistoryClosedReason::AccountClosed);
                            } else {
                                owner.refresh_reading();
                            }
                        }
                        res = reading.changed() => {
                            if res.is_err() {
                                owner.reading_lock().close(HistoryClosedReason::AccountClosed);
                            } else {
                                owner.refresh_reading();
                            }
                        }
                        res = catch_up.changed() => {
                            if res.is_err() {
                                owner.reading_lock().close(HistoryClosedReason::AccountClosed);
                            } else {
                                owner.refresh_reading();
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
        self.inner.refresh_reading();
        Ok(())
    }
}
