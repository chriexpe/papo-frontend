//! Bounded oldest-prefix eviction and coverage changes; global victim ordering and physical accounting live above.
//! Eviction moves a run's first boundary but never its identity, which is boundary-independent.

use super::{
    HistoryError,
    model::Model,
    persisted_layout::structural_layout,
    persistence::{State, Stored},
};
use crate::{AccountCache, Session, account_cache::history::HistoryKey, generation::Epoch};
use std::sync::Arc;
#[cfg(test)]
mod tests;

impl Model {
    /// Retire at most twenty oldest chronological entries without manufacturing deletion tombstones.
    pub(super) fn evict_prefix(&mut self) -> Result<Vec<[u8; 16]>, HistoryError> {
        self.drop_leading_runs_without_messages()?;
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
            // The anchor proved the space before the evicted prefix, not before the new first.
            self.chunks[0].oldest_anchor = None;
        } else {
            self.chunks.remove(0);
            if let Some(next) = self.chunks.first_mut() {
                next.oldest_complete = false;
            }
        }
        self.empty_generation = 0;
        if self.chunks.is_empty() {
            self.server_live_generation = 0;
            self.live_arrival_generation = 0;
            self.live_arrival_floor = None;
        }
        Ok(deleted)
    }
}

impl Model {
    /// A leading run holding no message (an anchor-only proof, or one whose messages were all
    /// deleted) would stop eviction from ever reaching the next run. Its proof is dropped with
    /// it, as an evicted run's is. Only held entries decide, so a held message is never left
    /// outside every run. Runs are kept while no message is held, so a turn that evicts
    /// nothing changes nothing.
    fn drop_leading_runs_without_messages(&mut self) -> Result<(), HistoryError> {
        if self.order.is_empty() {
            return Ok(());
        }
        while let Some(chunk) = self.chunks.first() {
            let (first, last) = (chunk.first.key()?, chunk.last.key()?);
            if self.order.range(first..=last).next().is_some() {
                break;
            }
            self.chunks.remove(0);
            if let Some(next) = self.chunks.first_mut() {
                next.oldest_complete = false;
            }
        }
        Ok(())
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
            // Admission and lookup are atomic; a channel this session does not admit is never
            // given an incarnation here, only its existing one (quarantined) is reused.
            let history = &session.inner.history;
            let (model, allowed) = match history.verified_channel(self.channel).await {
                Ok(model) => (model, true),
                Err(_) => match history.existing(self.channel).await {
                    Some(model) => (model, false),
                    None => return self.evict_cold().await,
                },
            };
            let mut state = model.state.lock().await;
            if allowed {
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
                let published = session
                    .inner
                    .epochs
                    .apply(epoch.generation, || model.publish(&state, epoch.generation))?;
                // A sealed incarnation delivers nothing; its eviction is still durable.
                match published {
                    Err(error) if !error.is_sealed() => return Err(error),
                    _ => {}
                }
            }
            return Ok(changed.then_some(state.last_write_rows));
        }
        self.evict_cold().await
    }

    async fn evict_cold(&mut self) -> Result<Option<usize>, HistoryError> {
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
    /// The channel's current set, or `None` when there is nothing to trim. A set in an older
    /// layout holds nothing this build can trim; it is retired (only if still current), so
    /// the channel's next authorized open rebuilds its history in a fresh set.
    async fn load(&self) -> Result<Option<State>, HistoryError> {
        let channel = *self.channel.as_bytes();
        let Some(data) = self.cache.retention_data(channel).await? else {
            return Ok(None);
        };
        let set_id = data.set_id;
        let stored = tokio::task::spawn_blocking(move || super::persistence::decode(data))
            .await
            .map_err(|_| crate::ClientError::Executor)??;
        match stored {
            Stored::Current(state) => Ok(Some(*state)),
            Stored::Outdated => {
                self.cache.retention_retire(channel, set_id).await?;
                Ok(None)
            }
        }
    }
}
