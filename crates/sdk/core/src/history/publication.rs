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
    keys: imbl::Vector<super::ordering::Key>,
    messages: imbl::Vector<Arc<Message>>,
    revision: u64,
    pub(super) payload_bytes: usize,
    counters: Arc<super::diagnostics::Counters>,
    updates: Option<watch::Sender<HistoryUpdate>>,
}

#[derive(Clone)]
struct Payload {
    revision: u64,
    key: super::ordering::Key,
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
        self.counters = Arc::clone(&state.counters);
        let version = self.next_version(self.snapshot.as_ref().map(|snapshot| snapshot.version))?;
        let same = self.incarnation == Some((state.cache_instance, state.set_id))
            && self.snapshot.is_some();
        let committed = same
            && self.revision == state.committed_from
            && state.committed_revision == state.model.revision;
        let ids: Vec<_> = if committed {
            state.changed.clone()
        } else if same && !state.model.dirty.is_empty() {
            state.model.dirty.iter().copied().collect()
        } else if same && self.revision == state.model.revision {
            Vec::new()
        } else {
            self.payloads
                .keys()
                .copied()
                .chain(state.model.order.values().copied())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        self.counters
            .publication_entries
            .fetch_add(ids.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.apply_changes(state, ids, same)?;
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
            messages: self.messages.clone(),
            chunks,
            gaps,
            empty_complete: state.model.order.is_empty()
                && state.model.empty_generation == generation.get(),
        });
        self.revision = state.model.revision;
        self.incarnation = Some((state.cache_instance, state.set_id));
        self.snapshot = Some(Arc::clone(&snapshot));
        self.counters
            .publications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.deliver(snapshot);
        Ok(())
    }

    fn apply_changes(
        &mut self,
        state: &State,
        ids: Vec<[u8; 16]>,
        same: bool,
    ) -> Result<(), HistoryError> {
        let mut ordered = ids
            .into_iter()
            .map(|id| {
                let key = state
                    .model
                    .entries
                    .get(&id)
                    .and_then(|entry| entry.point.as_ref())
                    .map(super::ordering::Point::key)
                    .transpose()?;
                Ok::<_, HistoryError>((key, id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Back-pagination should push a batch at the front, not repeatedly split and
        // rejoin the persistent tree at indexes 1..limit. Forward batches append in order.
        let prepend = self.keys.front().is_some_and(|first| {
            ordered
                .iter()
                .filter(|(_, id)| !self.payloads.contains_key(id))
                .filter_map(|(key, _)| *key)
                .all(|key| key < *first)
        });
        if prepend {
            ordered.sort_unstable_by(|a, b| b.cmp(a));
        } else {
            ordered.sort_unstable();
        }
        for (_, id) in ordered {
            self.apply_entry(state, id, same)?;
        }
        Ok(())
    }

    fn apply_entry(&mut self, state: &State, id: [u8; 16], same: bool) -> Result<(), HistoryError> {
        let entry = state
            .model
            .entries
            .get(&id)
            .filter(|entry| entry.timeline && !entry.deleted);
        let Some(entry) = entry else {
            if let Some(old) = self.payloads.remove(&id) {
                let index = self
                    .keys
                    .binary_search(&old.key)
                    .map_err(|_| CacheError::InvalidData)?;
                self.keys.remove(index);
                self.messages.remove(index);
                self.payload_bytes = self.payload_bytes.saturating_sub(old.bytes.len());
            }
            return Ok(());
        };
        let message = entry.message.as_ref().ok_or(CacheError::InvalidData)?;
        let key = entry.point.as_ref().ok_or(CacheError::InvalidData)?.key()?;
        let previous = self.payloads.get(&id);
        if same && previous.is_some_and(|old| old.revision == entry.revision && old.key == key) {
            return Ok(());
        }
        let bytes = serde_json::to_vec(message).map_err(|_| CacheError::InvalidData)?;
        self.counters
            .publication_encodes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let payload = if let Some(old) = previous.filter(|old| old.bytes.as_ref() == bytes) {
            Payload {
                revision: entry.revision,
                key,
                ..old.clone()
            }
        } else {
            Payload {
                revision: entry.revision,
                key,
                bytes: bytes.into(),
                message: Arc::new(message.clone()),
            }
        };
        if let Some(old) = previous {
            if old.key != key {
                return Err(CacheError::InvalidData.into());
            }
            let index = self
                .keys
                .binary_search(&key)
                .map_err(|_| CacheError::InvalidData)?;
            if !Arc::ptr_eq(&self.messages[index], &payload.message) {
                self.messages.set(index, Arc::clone(&payload.message));
            }
            self.payload_bytes = self.payload_bytes.saturating_sub(old.bytes.len());
        } else {
            let index = self
                .keys
                .binary_search(&key)
                .err()
                .ok_or(CacheError::InvalidData)?;
            self.keys.insert(index, key);
            self.messages.insert(index, Arc::clone(&payload.message));
        }
        self.payload_bytes = self.payload_bytes.saturating_add(payload.bytes.len());
        self.payloads.insert(id, payload);
        Ok(())
    }

    /// Normal channel updates remain consecutive. A newly hydrated publication starts above
    /// every version accepted by this account, preventing old snapshot stamps from aliasing
    /// a same-generation model recreated after idle eviction, without retaining per-ID clocks.
    fn next_version(&self, previous: Option<u64>) -> Result<u64, HistoryError> {
        use std::sync::atomic::Ordering::Relaxed;
        if let Some(previous) = previous {
            let version = previous.checked_add(1).ok_or(HistoryError::InvalidPage)?;
            self.counters.version_high_water.fetch_max(version, Relaxed);
            Ok(version)
        } else {
            self.counters
                .version_high_water
                .try_update(Relaxed, Relaxed, |version| version.checked_add(1))
                .map(|previous| previous + 1)
                .map_err(|_| HistoryError::InvalidPage)
        }
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
            version: self.next_version(Some(previous.version))?,
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
            self.keys.clear();
            self.messages.clear();
            self.payload_bytes = 0;
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
