//! Lock-contained window projection and subscription publication; asynchronous paging lives separately.

use imbl::Vector;
use papo_sdk::{
    HistoryClosedReason, HistoryCursor, HistorySnapshot, HistorySubscription, HistoryUpdate,
    Message, ReadingSnapshot, Uuid,
};
use std::sync::Arc;
use tokio::sync::watch;

use super::{TimelineConfig, TimelineError};
use crate::{
    list_subscription::Publisher,
    timeline_item::TimelineItem,
    timeline_window::{self, Bounds},
};

#[cfg(test)]
mod tests;

pub(super) struct WindowState {
    pub history: watch::Receiver<HistoryUpdate>,
    pub reading: watch::Receiver<Arc<ReadingSnapshot>>,
    pub raw: Arc<HistorySnapshot>,
    pub outgoing: Option<watch::Receiver<Arc<papo_sdk::OutgoingSnapshot>>>,
    outgoing_version: u64,
    pub bounds: Bounds,
    pub older_cursor: Option<HistoryCursor>,
    pub newer_cursor: Option<HistoryCursor>,
    pub loading: [bool; 2],
    pub publisher: Publisher<Arc<TimelineItem>, papo_sdk::Generation, HistoryClosedReason>,
    channel: Uuid,
    reading_version: u64,
    closed: Option<HistoryClosedReason>,
}

impl WindowState {
    pub fn new(
        subscription: HistorySubscription,
        reading: watch::Receiver<Arc<ReadingSnapshot>>,
        messages: Vec<Arc<Message>>,
        live: bool,
        channel: Uuid,
        config: &TimelineConfig,
    ) -> Result<Self, TimelineError> {
        let reading_version = reading.borrow().version;
        // A message can arrive after an empty page but before the atomic raw subscription.
        // Its captured baseline is already marked seen, so initialization must include it.
        let messages = if live && messages.is_empty() {
            subscription
                .snapshot
                .messages
                .iter()
                .rev()
                .take(usize::from(config.page_size))
                .cloned()
                .collect()
        } else {
            messages
        };
        let mut state = Self {
            bounds: Bounds {
                lower: None,
                upper: None,
                live,
            },
            publisher: Publisher::new(1, subscription.snapshot.generation, Vector::new()),
            raw: subscription.snapshot,
            outgoing: None,
            outgoing_version: 0,
            history: subscription.updates,
            reading_version,
            reading,
            channel,
            older_cursor: None,
            newer_cursor: None,
            loading: [false; 2],
            closed: None,
        };
        for message in messages {
            state.include(&message.cursor().ok_or(TimelineError::InvalidCursor)?)?;
        }
        state.publisher.items = state.project(config)?;
        Ok(state)
    }

    pub fn include(&mut self, cursor: &HistoryCursor) -> Result<(), TimelineError> {
        let point = timeline_window::key(cursor)?;
        if self.bounds.lower.is_none_or(|old| point < old) {
            self.bounds.lower = Some(point);
            self.older_cursor = Some(cursor.clone());
        }
        if self.bounds.upper.is_none_or(|old| point > old) {
            self.bounds.upper = Some(point);
            self.newer_cursor = Some(cursor.clone());
        }
        Ok(())
    }

    fn project(&self, config: &TimelineConfig) -> Result<Vector<Arc<TimelineItem>>, TimelineError> {
        let reading = self.reading.borrow();
        let read = (reading.generation == self.raw.generation)
            .then(|| reading.channels.get(&self.channel))
            .flatten();
        let mut items = timeline_window::project(
            &self.raw,
            &self.bounds,
            read,
            self.loading,
            config.offset_provider.as_ref(),
            &self.publisher.items,
        )?;
        if self.bounds.live
            && let Some(outgoing) = &self.outgoing
        {
            super::echoes::append(
                &mut items,
                &outgoing.borrow(),
                self.channel,
                &self.publisher.items,
                &self.raw,
            );
        }
        Ok(items)
    }

    pub fn reproject(&mut self, config: &TimelineConfig, reset: bool) -> Result<(), TimelineError> {
        let version = self
            .publisher
            .version
            .checked_add(1)
            .ok_or(TimelineError::VersionExhausted)?;
        let items = self.project(config)?;
        self.publisher
            .publish(version, self.raw.generation, items, reset, |item| {
                item.identity()
            });
        Ok(())
    }

    pub fn refresh(&mut self, config: &TimelineConfig) -> Result<(), TimelineError> {
        if let Some(reason) = self.closed {
            return Err(TimelineError::Closed(reason));
        }
        let update = self.history.borrow_and_update().clone();
        let next = match update {
            HistoryUpdate::Reset(snapshot) => snapshot,
            HistoryUpdate::Closed(reason) => {
                self.close(reason);
                return Err(TimelineError::Closed(reason));
            }
            _ => return Err(TimelineError::InvalidCursor),
        };
        let reading_version = self.reading.borrow_and_update().version;
        let outgoing_version = self
            .outgoing
            .as_mut()
            .map_or(0, |rx| rx.borrow_and_update().version);
        if self.raw.version == next.version
            && self.raw.generation == next.generation
            && self.reading_version == reading_version
            && self.outgoing_version == outgoing_version
        {
            return Ok(());
        }
        let reset = self.raw.generation != next.generation
            || (self.raw.version != next.version
                && self.raw.version.checked_add(1) != Some(next.version));
        self.raw = next;
        self.reading_version = reading_version;
        self.outgoing_version = outgoing_version;
        if self.bounds.lower.is_none() {
            let cursors: Vec<_> = self
                .raw
                .messages
                .iter()
                .rev()
                .take(usize::from(config.page_size))
                .filter_map(|message| message.cursor())
                .collect();
            for cursor in cursors {
                self.include(&cursor)?;
            }
        }
        if self.bounds.live
            && let Some(cursor) = self
                .raw
                .messages
                .last()
                .and_then(|message| message.cursor())
        {
            self.newer_cursor = Some(cursor.clone());
            self.bounds.upper = Some(timeline_window::key(&cursor)?);
        }
        self.reproject(config, reset)
    }

    pub fn close(&mut self, reason: HistoryClosedReason) {
        if self.closed.is_none() {
            self.closed = Some(reason);
            self.publisher.close(reason);
        }
    }
}
