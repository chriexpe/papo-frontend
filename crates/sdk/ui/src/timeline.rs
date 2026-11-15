//! Independent observable channel windows over public SDK history/read APIs; transport and durable policy stay in core.

mod echoes;
mod paging;
mod state;
mod subscription;
pub use paging::TimelinePage;

use crate::timeline_item::{TimelineDirection, TimelineItem, UtcOffset, UtcOffsetProvider};
use eyeball_im::VectorDiff;
use futures_util::Stream;
use imbl::Vector;
use papo_sdk::{
    Generation, HistoryClosedReason, HistoryCursor, HistoryError, HistoryHandle, HistoryOrder,
    HistoryQuery, ReadingError, Session, Uuid,
};
use state::WindowState;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};

#[cfg(test)]
mod tests;

/// Per-window configuration. UTC is the default; clients can supply their own historical offsets.
#[derive(Clone)]
pub struct TimelineConfig {
    /// Initial and subsequent page size, in messages, from 1 through 100.
    pub page_size: u8,
    /// Maximum queued batches per subscriber before one coherent Reset; default 64.
    pub subscriber_capacity: usize,
    /// Offset by message timestamp, never by wall-clock now.
    pub offset_provider: Arc<dyn UtcOffsetProvider>,
}

impl Default for TimelineConfig {
    fn default() -> Self {
        Self {
            page_size: 30,
            subscriber_capacity: 64,
            offset_provider: Arc::new(|_| UtcOffset::default()),
        }
    }
}

/// Sanitized window failures; clients translate these statuses rather than displaying diagnostics.
#[derive(Debug)]
#[non_exhaustive]
pub enum TimelineError {
    /// Canonical history, generation or storage failure.
    History(HistoryError),
    /// Durable reading policy or upload-admission failure.
    Reading(ReadingError),
    /// Durable outgoing state failed to load.
    Outgoing(papo_sdk::OutgoingError),
    /// Exact cursor lacked a valid timestamp or UUID.
    InvalidCursor,
    /// Page size or queue capacity was zero or outside the protocol limit.
    InvalidConfig,
    /// The displayed window version or generation is no longer current.
    StaleSnapshot,
    /// The exact focused target was absent, deleted, or outside the server's returned neighborhood.
    FocusMissing,
    /// Lifecycle revocation is terminal for this window.
    Closed(HistoryClosedReason),
    /// Publication version cannot advance safely.
    VersionExhausted,
    /// A page in this direction is already in flight.
    Busy,
    /// An in-flight hourglass seek was cancelled cleanly.
    Cancelled,
}
impl std::fmt::Display for TimelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::History(error) => error.fmt(f),
            Self::Reading(error) => error.fmt(f),
            Self::Outgoing(error) => error.fmt(f),
            _ => f.write_str("timeline operation rejected"),
        }
    }
}
impl std::error::Error for TimelineError {}
impl From<HistoryError> for TimelineError {
    fn from(error: HistoryError) -> Self {
        Self::History(error)
    }
}
impl From<ReadingError> for TimelineError {
    fn from(error: ReadingError) -> Self {
        Self::Reading(error)
    }
}

/// A complete window transition. Apply every diff in order before recording its version/generation.
#[derive(Clone)]
pub struct TimelineUpdate {
    /// New window version after the complete batch.
    pub version: u64,
    /// Connection generation of this window's raw history.
    pub generation: Generation,
    /// Identity-preserving edits, or one Reset after overflow/coalesced source versions.
    pub diffs: Vec<VectorDiff<Arc<TimelineItem>>>,
}

/// Every active subscription receives one terminal typed close, then ends permanently.
#[derive(Clone)]
#[non_exhaustive]
pub enum TimelineEvent {
    /// One atomic batch advancing this window's version and generation.
    Update(TimelineUpdate),
    /// Terminal account/channel revocation; no later batch can revive this subscription.
    Closed(HistoryClosedReason),
}

/// Pull-driven bounded window updates; no dedicated task or runtime is created.
pub type TimelineStream = Pin<Box<dyn Stream<Item = TimelineEvent> + Send>>;

/// Baseline and registration captured under one window lock, after observing current SDK state.
pub struct TimelineSubscription {
    /// Version represented by `items`.
    pub version: u64,
    /// Generation captured with that same version.
    pub generation: Generation,
    /// View-ready items in ascending chronological order.
    pub items: Vector<Arc<TimelineItem>>,
    /// Strictly subsequent window changes, followed by typed closure.
    pub stream: TimelineStream,
}

/// One window; cloning shares its bounds. Open another handle for independent pagination.
/// All handles for an account/channel read the same canonical SDK state.
#[derive(Clone)]
pub struct Timeline {
    pub(crate) inner: Arc<Owner>,
}

pub(crate) struct Owner {
    pub(crate) session: Session,
    pub(crate) history: HistoryHandle,
    config: TimelineConfig,
    observer: papo_sdk::ReadingObserver,
    state: Mutex<WindowState>,
    reading: Mutex<crate::timeline_reading::ReadingOverlay>,
}

impl Owner {
    fn lock(&self) -> std::sync::MutexGuard<'_, WindowState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn reading_lock(
        &self,
    ) -> std::sync::MutexGuard<'_, crate::timeline_reading::ReadingOverlay> {
        self.reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn refresh_reading(&self) -> Result<(), TimelineError> {
        let mut overlay = self.reading_lock();
        overlay.refresh(&self.session, self.history.channel())
    }

    pub(crate) fn record_exposed_and_refresh(
        &self,
        exposed: Option<papo_sdk::ReadPoint>,
    ) -> Result<(), TimelineError> {
        let mut overlay = self.reading_lock();
        overlay.record_exposed(&self.session, self.history.channel(), exposed)
    }
}

impl Timeline {
    /// Open a live window whose newest bound follows future messages. Fetching never acknowledges.
    /// # Errors
    /// Returns invalid configuration, revoked/unverified history, transport or storage failures.
    pub async fn live(
        session: Session,
        channel: Uuid,
        config: TimelineConfig,
    ) -> Result<Self, TimelineError> {
        Self::open(session, channel, None, config).await
    }

    /// Open an independent focused neighborhood from an exact pin, reply or search cursor.
    /// Detached payloads alone never become chronological coverage or read acknowledgements.
    /// # Errors
    /// Returns invalid cursor/configuration, missing focused target or canonical history failures.
    pub async fn focused(
        session: Session,
        channel: Uuid,
        cursor: HistoryCursor,
        config: TimelineConfig,
    ) -> Result<Self, TimelineError> {
        Self::open(session, channel, Some(cursor), config).await
    }

    async fn open(
        session: Session,
        channel: Uuid,
        focus: Option<HistoryCursor>,
        config: TimelineConfig,
    ) -> Result<Self, TimelineError> {
        if config.page_size == 0 || config.page_size > 100 || config.subscriber_capacity == 0 {
            return Err(TimelineError::InvalidConfig);
        }
        if let Some(cursor) = &focus {
            crate::timeline_window::key(cursor)?;
        }
        let history = session.history(channel).await?;
        let first = history
            .page(HistoryQuery {
                cursor: focus.as_ref(),
                order: HistoryOrder::Desc,
                limit: Some(config.page_size),
            })
            .await?;
        let mut messages = first.messages;
        if let Some(focus) = &focus {
            // The pin has no GET-by-ID route. Fetching forward from its nearest older row includes
            // the target using only exact server-supplied cursors, including equal-time UUID ties.
            let before = messages.first().and_then(|message| message.cursor());
            let after = history
                .page(HistoryQuery {
                    cursor: before.as_ref(),
                    order: HistoryOrder::Asc,
                    limit: Some(config.page_size),
                })
                .await?;
            messages.extend(after.messages);
            if !messages
                .iter()
                .any(|message| message.cursor().as_ref() == Some(focus))
            {
                return Err(TimelineError::FocusMissing);
            }
        }
        let subscription = history.subscribe().await?;
        let reading = session.subscribe_reading();
        let catch_up = session.subscribe_catch_up();
        let observer = session.reading_observer();
        let mut state = WindowState::new(
            subscription,
            reading.clone(),
            messages,
            focus.is_none(),
            channel,
            &config,
        )?;
        if focus.is_none() {
            state.outgoing = Some(
                session
                    .outgoing()
                    .subscribe()
                    .await
                    .map_err(TimelineError::Outgoing)?,
            );
        }
        state.refresh(&config)?;
        let reading_overlay = crate::timeline_reading::ReadingOverlay::new(
            &session,
            channel,
            Arc::clone(&state.raw),
            state.history.clone(),
            catch_up,
        )?;
        Ok(Self {
            inner: Arc::new(Owner {
                session,
                history,
                config,
                observer,
                state: Mutex::new(state),
                reading: Mutex::new(reading_overlay),
            }),
        })
    }

    /// Report only message IDs actually visible in this exact window baseline, paired with scroll
    /// continuity. Clients must not call while backgrounded or covered.
    /// The SDK acknowledges only after its FULL durable commit; this never marks unseen rows read.
    /// # Errors
    /// Returns stale view stamps, invalid visible IDs, lifecycle or durable reading failures.
    pub async fn report_visible(
        &self,
        version: u64,
        generation: Generation,
        ids: &[Uuid],
        continuity: papo_sdk::VisibilityContinuity,
    ) -> Result<(), TimelineError> {
        let (history_version, max_reported) = {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            if state.publisher.version != version || state.publisher.generation != generation {
                return Err(TimelineError::StaleSnapshot);
            }
            let mut max_reported: Option<papo_sdk::ReadPoint> = None;
            for id in ids {
                let Some(message) =
                    state
                        .publisher
                        .items
                        .iter()
                        .find_map(|item| match item.as_ref() {
                            TimelineItem::Message {
                                id: current,
                                message,
                            } if current == id => Some(message),
                            _ => None,
                        })
                else {
                    return Err(TimelineError::StaleSnapshot);
                };
                if let Some(point) = crate::timeline_arrivals::message_point(message)? {
                    max_reported = Some(max_reported.map_or(point, |current| current.max(point)));
                }
            }
            (state.raw.version, max_reported)
        };
        self.inner
            .observer
            .report_visible(
                self.inner.history.channel(),
                generation,
                history_version,
                ids,
                continuity,
            )
            .await?;
        self.inner.lock().refresh(&self.inner.config)?;
        self.inner.record_exposed_and_refresh(max_reported)?;
        Ok(())
    }
}
