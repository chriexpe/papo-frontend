//! Lock-owned accepted raw snapshots, payload identity and sticky closure; public consumption lives in subscription.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::{HistoryOrder, wire::Message};
use tokio::sync::watch;

use crate::{CacheError, Generation};

use super::{
    HistoryClosedReason, HistoryError, HistoryGap,
    channel::Channel,
    persistence::State,
    subscription::{HistoryChunk, HistorySnapshot, HistorySubscription, HistoryUpdate},
};

#[derive(Default)]
pub(super) struct Publication {
    pub closed: Option<HistoryClosedReason>,
    pub snapshot: Option<Arc<HistorySnapshot>>,
    incarnation: Option<(Option<u64>, Option<i64>)>,
    payloads: BTreeMap<[u8; 16], Payload>,
    updates: Option<watch::Sender<HistoryUpdate>>,
}

#[derive(Clone)]
struct Payload {
    revision: u64,
    bytes: Arc<[u8]>,
    message: Arc<Message>,
}

impl Publication {
    pub fn check(&self) -> Result<(), HistoryError> {
        self.closed
            .map_or(Ok(()), |reason| Err(HistoryError::Closed(reason)))
    }

    fn publish(&mut self, state: &State, generation: Generation) -> Result<(), HistoryError> {
        self.check()?;
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.generation > generation)
        {
            return Err(crate::ClientError::Superseded.into());
        }
        let version = self
            .snapshot
            .as_ref()
            .map_or(Some(1), |snapshot| snapshot.version.checked_add(1))
            .ok_or(HistoryError::InvalidPage)?;
        let mut payloads = BTreeMap::new();
        let mut messages = Vec::with_capacity(state.model.order.len());
        for id in state.model.order.values() {
            let entry = state.model.entries.get(id).ok_or(CacheError::InvalidData)?;
            if !entry.timeline || entry.deleted {
                return Err(CacheError::InvalidData.into());
            }
            let message = entry.message.as_ref().ok_or(CacheError::InvalidData)?;
            let previous = self.payloads.get(id);
            let payload = if let Some(previous) = previous.filter(|previous| {
                previous.revision == entry.revision
                    && self.incarnation == Some((state.cache_instance, state.set_id))
            }) {
                previous.clone()
            } else {
                let bytes = serde_json::to_vec(message).map_err(|_| CacheError::InvalidData)?;
                if let Some(previous) = previous.filter(|previous| previous.bytes.as_ref() == bytes)
                {
                    Payload {
                        revision: entry.revision,
                        ..previous.clone()
                    }
                } else {
                    Payload {
                        revision: entry.revision,
                        bytes: bytes.into(),
                        message: Arc::new(message.clone()),
                    }
                }
            };
            messages.push(Arc::clone(&payload.message));
            payloads.insert(*id, payload);
        }
        let mut chunks = Vec::with_capacity(state.model.chunks.len());
        let mut gaps = Vec::new();
        for chunk in &state.model.chunks {
            chunks.push(HistoryChunk {
                first: chunk.first.cursor(),
                last: chunk.last.cursor(),
                oldest_complete: chunk.oldest_complete,
                newest_complete: chunk.newest_complete,
                generation: (chunk.generation != 0).then_some(Generation(chunk.generation)),
                live_generation: (chunk.live_generation != 0)
                    .then_some(Generation(chunk.live_generation)),
            });
            for (complete, point, order) in [
                (chunk.oldest_complete, &chunk.first, HistoryOrder::Desc),
                (chunk.newest_complete, &chunk.last, HistoryOrder::Asc),
            ] {
                if !complete {
                    gaps.push(HistoryGap {
                        cursor: point.cursor(),
                        order,
                    });
                }
            }
        }
        let snapshot = Arc::new(HistorySnapshot {
            version,
            generation,
            messages,
            chunks,
            gaps,
            empty_complete: state.model.order.is_empty()
                && state.model.empty_generation == generation.get(),
        });
        self.payloads = payloads;
        self.incarnation = Some((state.cache_instance, state.set_id));
        self.snapshot = Some(Arc::clone(&snapshot));
        self.deliver(snapshot);
        Ok(())
    }

    fn deliver(&mut self, snapshot: Arc<HistorySnapshot>) {
        let update = HistoryUpdate::Reset(snapshot);
        if let Some(updates) = &self.updates {
            updates.send_replace(update);
        } else {
            self.updates = Some(watch::channel(update).0);
        }
    }

    fn restamp(&mut self, generation: Generation) -> Result<(), HistoryError> {
        self.check()?;
        let Some(previous) = &self.snapshot else {
            return Ok(());
        };
        if previous.generation > generation {
            return Err(crate::ClientError::Superseded.into());
        }
        if previous.generation == generation {
            return Ok(());
        }
        let snapshot = Arc::new(HistorySnapshot {
            version: previous
                .version
                .checked_add(1)
                .ok_or(HistoryError::InvalidPage)?,
            generation,
            empty_complete: false,
            ..(**previous).clone()
        });
        self.snapshot = Some(Arc::clone(&snapshot));
        self.deliver(snapshot);
        Ok(())
    }

    pub fn close(&mut self, reason: HistoryClosedReason) {
        if self.closed.is_none() {
            self.closed = Some(reason);
            self.payloads.clear();
            self.snapshot = None;
            if let Some(updates) = &self.updates {
                updates.send_replace(HistoryUpdate::Closed(reason));
            }
        }
    }

    fn subscribe(&self) -> Result<HistorySubscription, HistoryError> {
        self.check()?;
        let snapshot = self.snapshot.clone().ok_or(HistoryError::Unverified)?;
        let mut updates = self
            .updates
            .as_ref()
            .ok_or(HistoryError::Unverified)?
            .subscribe();
        updates.mark_unchanged();
        Ok(HistorySubscription { snapshot, updates })
    }
}

impl Channel {
    /// Caller holds the canonical state lock until publication finishes.
    pub fn publish(&self, state: &State, generation: Generation) -> Result<(), HistoryError> {
        self.publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .publish(state, generation)?;
        self.changes.send_replace(());
        Ok(())
    }

    /// Only cache incarnation or generation changes may replace an already accepted publication.
    pub fn initialize(&self, state: &State, generation: Generation) -> Result<(), HistoryError> {
        let mut publication = self
            .publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        publication.check()?;
        if publication.incarnation != Some((state.cache_instance, state.set_id))
            || publication.snapshot.is_none()
        {
            publication.publish(state, generation)?;
            self.changes.send_replace(());
        } else if publication
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.generation != generation)
        {
            publication.restamp(generation)?;
            self.changes.send_replace(());
        }
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.opens.load(std::sync::atomic::Ordering::Relaxed) > 0
            || self
                .publication
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .updates
                .as_ref()
                .is_some_and(|updates| updates.receiver_count() > 0)
    }

    pub(super) fn generation_changed(&self, generation: Generation) -> Result<(), HistoryError> {
        self.publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .restamp(generation)?;
        self.changes.send_replace(());
        Ok(())
    }

    pub fn subscribe(&self) -> Result<HistorySubscription, HistoryError> {
        self.publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .subscribe()
    }
}
