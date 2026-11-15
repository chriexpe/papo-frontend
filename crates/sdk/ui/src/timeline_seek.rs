//! Cancellable hourglass navigation resolving unloaded prefixes before opening a focused timeline.
//! Reading overlay projections live in `timeline_reading`.

use std::sync::Arc;

use chrono::SecondsFormat;
use papo_sdk::{HistoryCursor, HistoryOrder, HistoryQuery, HistorySnapshot, ReadPoint, Uuid};
use tokio::sync::watch;

use super::{Timeline, TimelineConfig, TimelineError};
use crate::timeline::Owner;

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
            .set_seeking(&self.owner.session, self.channel, -1);
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
        for _ in 0..512 {
            if *cancel_rx.borrow() {
                return Err(TimelineError::Cancelled);
            }
            let raw = owner.history.subscribe().await?.snapshot;
            let view = owner.session.catch_up_view(channel, &raw)?;
            if !view.has_catch_up {
                return Err(TimelineError::FocusMissing);
            }
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
                cursor_override = view
                    .start_after
                    .map(|pt| point_to_cursor(&raw, pt))
                    .transpose()?;
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
                let latest_view = owner.session.catch_up_view(channel, &latest)?;
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

fn point_to_cursor(
    raw: &HistorySnapshot,
    point: ReadPoint,
) -> Result<HistoryCursor, TimelineError> {
    if let Some(cursor) = crate::timeline_arrivals::find_message_cursor(raw, point)? {
        return Ok(cursor);
    }
    let created = chrono::DateTime::from_timestamp_micros(point.created_at_us)
        .ok_or(TimelineError::InvalidCursor)?
        .to_rfc3339_opts(SecondsFormat::Micros, true);
    Ok(HistoryCursor {
        created_at: created,
        id: point.message_id.to_string(),
    })
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
            if !overlay.state.catch_up.available {
                return Err(TimelineError::FocusMissing);
            }
            overlay.set_seeking(&self.inner.session, channel, 1);
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
