//! Independent observable channel windows over public SDK history/read APIs; transport and durable policy stay in core.

mod paging;
mod state;
pub use paging::TimelinePage;

use crate::{
    list_subscription,
    timeline_item::{TimelineDirection, TimelineItem, UtcOffset, UtcOffsetProvider},
};
use eyeball_im::VectorDiff;
use futures_util::{
    Stream, StreamExt,
    future::{self, Either},
    stream,
};
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
    /// The publication version cannot advance safely.
    VersionExhausted,
    /// A page in this direction is already in flight.
    Busy,
}
impl std::fmt::Display for TimelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::History(error) => error.fmt(f),
            Self::Reading(error) => error.fmt(f),
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
    inner: Arc<Owner>,
}

struct Owner {
    session: Session,
    history: HistoryHandle,
    config: TimelineConfig,
    state: Mutex<WindowState>,
}

impl Owner {
    fn lock(&self) -> std::sync::MutexGuard<'_, WindowState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        let state = WindowState::new(
            subscription,
            reading,
            messages,
            focus.is_none(),
            channel,
            &config,
        )?;
        Ok(Self {
            inner: Arc::new(Owner {
                session,
                history,
                config,
                state: Mutex::new(state),
            }),
        })
    }

    /// Capture `(version, items, stream)` under one lock. Slow subscriptions retain at most
    /// `subscriber_capacity` batches; overflow discards them and emits one current Reset.
    /// # Errors
    /// Returns a terminal closure, invalid authoritative cursor or exhausted version.
    pub fn subscribe(&self) -> Result<TimelineSubscription, TimelineError> {
        let mut state = self.inner.lock();
        state.refresh(&self.inner.config)?;
        let version = state.publisher.version;
        let generation = state.publisher.generation;
        let items = state.publisher.items.clone();
        let receiver = state
            .publisher
            .subscribe(self.inner.config.subscriber_capacity);
        let history = state.history.clone();
        let reading = state.reading.clone();
        let owner = Arc::clone(&self.inner);
        let changes = stream::unfold(
            (owner, receiver, history, reading),
            |(owner, mut receiver, mut history, mut reading)| async move {
                loop {
                    // A terminal refresh has already enqueued Closed; drain it rather than losing the reason.
                    let _ = owner.lock().refresh(&owner.config);
                    let wake = {
                        match future::select(
                            Box::pin(receiver.next()),
                            Box::pin(future::select(
                                Box::pin(history.changed()),
                                Box::pin(reading.changed()),
                            )),
                        )
                        .await
                        {
                            Either::Left((event, pending)) => {
                                drop(pending);
                                Either::Left(event)
                            }
                            Either::Right((change, pending)) => {
                                drop(pending);
                                let closed = match change {
                                    Either::Left((result, pending)) => {
                                        drop(pending);
                                        result.is_err()
                                    }
                                    Either::Right((result, pending)) => {
                                        drop(pending);
                                        result.is_err()
                                    }
                                };
                                Either::Right(closed)
                            }
                        }
                    };
                    match wake {
                        Either::Left(Some(event)) => {
                            let event = match event {
                                list_subscription::Event::Update {
                                    version,
                                    generation,
                                    diffs,
                                } => TimelineEvent::Update(TimelineUpdate {
                                    version,
                                    generation,
                                    diffs,
                                }),
                                list_subscription::Event::Closed(reason) => {
                                    TimelineEvent::Closed(reason)
                                }
                            };
                            return Some((event, (owner, receiver, history, reading)));
                        }
                        Either::Left(None) => return None,
                        Either::Right(closed) => {
                            if closed {
                                owner.lock().close(HistoryClosedReason::AccountClosed);
                            }
                        }
                    }
                }
            },
        );
        Ok(TimelineSubscription {
            version,
            generation,
            items,
            stream: Box::pin(changes.fuse()),
        })
    }

    /// Report only message IDs actually visible in this exact window baseline. Clients must not
    /// call while backgrounded or covered. An old window/core version or generation is rejected.
    /// The SDK acknowledges only after its FULL durable commit; this never marks unseen rows read.
    /// # Errors
    /// Returns stale view stamps, invalid visible IDs, lifecycle or durable reading failures.
    pub async fn report_visible(
        &self,
        version: u64,
        generation: Generation,
        ids: &[Uuid],
    ) -> Result<(), TimelineError> {
        let history_version = {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            if state.publisher.version != version || state.publisher.generation != generation {
                return Err(TimelineError::StaleSnapshot);
            }
            if ids.iter().any(|id| !state.publisher.items.iter().any(|item| matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if current == id))) { return Err(TimelineError::StaleSnapshot); }
            state.raw.version
        };
        self.inner
            .session
            .report_visible(
                self.inner.history.channel(),
                generation,
                history_version,
                ids,
            )
            .await?;
        self.inner.lock().refresh(&self.inner.config)?;
        Ok(())
    }
}
