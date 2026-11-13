//! Bounded oldest-prefix eviction and coverage changes; global victim ordering and physical accounting live above.

use super::{HistoryError, model::Model, persisted_layout::structural_layout, persistence::State};
use crate::{AccountCache, Session, account_cache::history::HistoryKey, generation::Epoch};
use std::sync::Arc;
#[cfg(test)]
mod tests;

impl Model {
    /// Retire at most twenty oldest chronological entries without manufacturing deletion tombstones.
    pub(super) fn evict_prefix(&mut self) -> Result<Vec<[u8; 16]>, HistoryError> {
        let Some(chunk) = self.chunks.first() else {
            return Ok(Vec::new());
        };
        let last = chunk.last.key()?;
        let keys: Vec<_> = self
            .order
            .range(..=last)
            .take(20)
            .map(|(key, id)| (*key, *id))
            .collect();
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let revision = self.next_revision()?;
        let mut deleted = Vec::new();
        for (key, id) in keys {
            self.order.remove(&key);
            let entry = self.entries.get_mut(&id).ok_or(HistoryError::InvalidPage)?;
            if entry.pinned {
                entry.timeline = false;
                entry.revision = revision;
                self.dirty.insert(id);
            } else {
                self.entries.remove(&id);
                self.dirty.remove(&id);
            }
            deleted.push(id);
        }
        if let Some((first, id)) = self
            .order
            .first_key_value()
            .filter(|(key, _)| **key <= last)
        {
            let point = self.entries[id]
                .point
                .clone()
                .ok_or(HistoryError::InvalidPage)?;
            if point.key()? != *first {
                return Err(HistoryError::InvalidPage);
            }
            self.chunks[0].first = point;
            self.chunks[0].oldest_complete = false;
        } else {
            self.chunks.remove(0);
            if let Some(next) = self.chunks.first_mut() {
                next.oldest_complete = false;
            }
        }
        self.empty_generation = 0;
        Ok(deleted)
    }
}

impl State {
    async fn evict(
        &mut self,
        cache: &AccountCache,
        channel: [u8; 16],
        epoch: &Epoch,
    ) -> Result<bool, HistoryError> {
        let old_order = self.model.order.len();
        let old_layout = structural_layout(&self.model)?;
        let deleted = self.model.evict_prefix()?;
        if self.model.order.len() == old_order {
            return Ok(false);
        }
        let next = structural_layout(&self.model)?;
        for ((table, id), _) in old_layout.iter().filter(|(key, _)| !next.contains_key(key)) {
            self.physical_deletes.push(HistoryKey {
                table: super::persisted_layout::partition(*table)?,
                id: *id,
            });
        }
        for id in &deleted {
            for table in [
                crate::account_cache::history::HistoryTable::Messages,
                crate::account_cache::history::HistoryTable::Items,
            ] {
                if table == crate::account_cache::history::HistoryTable::Messages
                    && self.model.entries.contains_key(id)
                {
                    continue;
                }
                self.physical_deletes.push(HistoryKey { table, id: *id });
            }
        }
        if let Err(error) = self.persist(cache, channel, epoch).await {
            self.reset();
            return Err(error);
        }
        for id in &deleted {
            if !self.model.entries.contains_key(id) {
                self.encoded_bytes = self
                    .encoded_bytes
                    .saturating_sub(self.entry_bytes.remove(id).unwrap_or(0));
            }
        }
        self.changed.extend(deleted);
        Ok(true)
    }
}

/// One candidate retains a cold model only until a live account owns the canonical state.
pub(crate) struct Retainer {
    cache: Arc<AccountCache>,
    channel: crate::Uuid,
    cold: Option<State>,
}
impl Retainer {
    pub(crate) fn new(cache: Arc<AccountCache>, channel: crate::Uuid) -> Self {
        Self {
            cache,
            channel,
            cold: None,
        }
    }
    pub(crate) async fn evict(
        &mut self,
        session: Option<Session>,
    ) -> Result<Option<usize>, HistoryError> {
        if let Some(session) = session {
            let epoch = session.inner.epochs.current();
            session.inner.epochs.validate(epoch.generation)?;
            let model = session.inner.history.channel(self.channel).await;
            let mut state = model.state.lock().await;
            if session.inner.history.is_verified(self.channel).await {
                state
                    .ensure(&self.cache, *self.channel.as_bytes(), epoch.generation)
                    .await?;
            } else if state.set_id.is_none() {
                let Some(cold) = self.load().await? else {
                    return Ok(None);
                };
                let mut cold = cold;
                cold.counters = Arc::clone(&state.counters);
                *state = cold;
                state.cache_instance = Some(self.cache.history_instance());
            }
            self.cold = None;
            let maintenance = Epoch {
                generation: crate::Generation(0),
                cancel: epoch.cancel.clone(),
            };
            let changed = state
                .evict(&self.cache, *self.channel.as_bytes(), &maintenance)
                .await?;
            state.retention_version = self.cache.retention_version(*self.channel.as_bytes());
            if changed {
                session
                    .inner
                    .epochs
                    .apply(epoch.generation, || model.publish(&state, epoch.generation))??;
            }
            return Ok(changed.then_some(state.last_write_rows));
        }
        if self.cold.is_none() {
            self.cold = self.load().await?;
        }
        let Some(state) = self.cold.as_mut() else {
            return Ok(None);
        };
        let epoch = Epoch {
            generation: crate::Generation(0),
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let changed = state
            .evict(&self.cache, *self.channel.as_bytes(), &epoch)
            .await?;
        Ok(changed.then_some(state.last_write_rows))
    }
    async fn load(&self) -> Result<Option<State>, HistoryError> {
        let Some(data) = self.cache.retention_data(*self.channel.as_bytes()).await? else {
            return Ok(None);
        };
        Ok(Some(
            tokio::task::spawn_blocking(move || super::persistence::decode(data))
                .await
                .map_err(|_| crate::ClientError::Executor)??,
        ))
    }
}
