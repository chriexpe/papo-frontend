//! Cancellable hourglass navigation resolving unloaded prefixes before opening a focused timeline.
//! Reading overlay projections live in `timeline_reading`.

use std::sync::Arc;

use papo_sdk::{HistoryCursor, HistoryOrder, HistoryQuery, Uuid};
use tokio::sync::watch;

use super::{Timeline, TimelineConfig, TimelineError};
use crate::timeline::Owner;

const MAX_CATCH_UP_SEEK_PAGES: usize = 512;

struct SeekCancel(watch::Sender<bool>);

impl Drop for SeekCancel {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct SeekingGuard {
    owner: Arc<Owner>,
    channel: Uuid,
}

impl Drop for SeekingGuard {
    fn drop(&mut self) {
        self.owner
            .reading_lock()
            .end_seeking(&self.owner.session, self.channel);
    }
}

/// Cancel-on-drop hourglass seek operation resolving unloaded prefixes before opening focused view.
pub struct CatchUpSeek {
    owner: Arc<Owner>,
    config: TimelineConfig,
    cancel: SeekCancel,
    cancel_rx: watch::Receiver<bool>,
    _guard: SeekingGuard,
}

impl CatchUpSeek {
    /// Cancels this hourglass seek immediately; further prefix pagination stops without erasing
    /// the channel's catch-up interval.
    pub fn cancel(&self) {
        self.cancel.0.send_replace(true);
    }

    /// Resolves any unloaded prefix starting after `start_after` and opens a focused timeline at
    /// the interval's canonical start target (never at the old read anchor).
    /// # Errors
    /// Returns `Cancelled`, `FocusMissing` if the interval closed, `InvalidConfig`, or history errors.
    pub async fn finish(self) -> Result<Timeline, TimelineError> {
        let Self {
            owner,
            config,
            cancel: _cancel,
            mut cancel_rx,
            _guard,
        } = self;
        let channel = owner.history.channel();
        let mut cursor_override: Option<HistoryCursor> = None;
        let mut initialized_cursor = false;
        for _ in 0..MAX_CATCH_UP_SEEK_PAGES {
            if *cancel_rx.borrow() {
                return Err(TimelineError::Cancelled);
            }
            let raw = owner.history.subscribe().await?.snapshot;
            let Some(view) = owner.session.catch_up_view(channel, &raw)? else {
                return Err(TimelineError::FocusMissing);
            };
            if !view.needs_history
                && let Some(target_cursor) = view.start_cursor
            {
                return open_focused_cancellable(
                    &owner,
                    channel,
                    target_cursor,
                    &config,
                    &mut cancel_rx,
                )
                .await;
            }
            if !initialized_cursor {
                cursor_override = match view.start_after {
                    Some(point) => Some(start_cursor(&raw, point)?),
                    None => None,
                };
                initialized_cursor = true;
            }
            let query = HistoryQuery {
                cursor: cursor_override.as_ref(),
                order: HistoryOrder::Asc,
                limit: Some(config.page_size),
            };
            let page = tokio::select! {
                biased;
                _ = cancel_rx.wait_for(|&c| c) => return Err(TimelineError::Cancelled),
                res = owner.history.page(query) => res?,
            };
            owner.refresh_reading()?;
            if page.messages.is_empty() || !page.has_more {
                let latest = owner.history.subscribe().await?.snapshot;
                let Some(latest_view) = owner.session.catch_up_view(channel, &latest)? else {
                    return Err(TimelineError::FocusMissing);
                };
                if let Some(target_cursor) = latest_view.start_cursor {
                    return open_focused_cancellable(
                        &owner,
                        channel,
                        target_cursor,
                        &config,
                        &mut cancel_rx,
                    )
                    .await;
                }
                return Err(TimelineError::FocusMissing);
            }
            if page.next_cursor.as_ref() == cursor_override.as_ref() {
                return Err(TimelineError::FocusMissing);
            }
            cursor_override = page.next_cursor;
        }
        Err(TimelineError::FocusMissing)
    }
}

/// The held message's exact server cursor when the start is held, otherwise one rebuilt from
/// its key (a deleted or evicted anchor still pages from the right place).
fn start_cursor(
    raw: &papo_sdk::HistorySnapshot,
    point: papo_sdk::MessageKey,
) -> Result<HistoryCursor, TimelineError> {
    let held = raw
        .coverage()?
        .resolve(point)?
        .index
        .and_then(|index| raw.messages[index].cursor());
    match held {
        Some(cursor) => Ok(cursor),
        None => Ok(point.to_cursor()?),
    }
}

async fn open_focused_cancellable(
    owner: &Arc<Owner>,
    channel: Uuid,
    target_cursor: HistoryCursor,
    config: &TimelineConfig,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<Timeline, TimelineError> {
    tokio::select! {
        biased;
        _ = cancel_rx.wait_for(|&c| c) => Err(TimelineError::Cancelled),
        res = Timeline::focused(
            owner.session.clone(),
            channel,
            target_cursor,
            config.clone(),
        ) => res,
    }
}

impl Timeline {
    /// Starts hourglass navigation toward the current catch-up interval's canonical start target.
    /// Dropping or calling `.cancel()` cancels pending pagination without erasing the interval.
    /// # Errors
    /// Returns closure, `FocusMissing` when no catch-up interval exists, or `InvalidConfig`.
    pub fn seek_catch_up(&self, config: TimelineConfig) -> Result<CatchUpSeek, TimelineError> {
        if config.page_size == 0 || config.page_size > 100 || config.subscriber_capacity == 0 {
            return Err(TimelineError::InvalidConfig);
        }
        let channel = self.inner.history.channel();
        {
            let mut overlay = self.inner.reading_lock();
            overlay.refresh(&self.inner.session, channel)?;
            if overlay.state.catch_up.is_none() {
                return Err(TimelineError::FocusMissing);
            }
            overlay.begin_seeking(&self.inner.session, channel);
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        Ok(CatchUpSeek {
            owner: Arc::clone(&self.inner),
            config,
            cancel: SeekCancel(cancel_tx),
            cancel_rx,
            _guard: SeekingGuard {
                owner: Arc::clone(&self.inner),
                channel,
            },
        })
    }
}
