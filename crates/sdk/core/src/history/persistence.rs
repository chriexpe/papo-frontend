//! Incremental model rows and atomic tombstone/coverage markers; cache owns revocation and physical purge.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AccountCache, CacheError, Generation,
    account_cache::history::{HistoryData, HistoryRow, HistoryTable},
};

use super::{
    HistoryError,
    diagnostics::Counters,
    model::{Entry, Model},
    persisted_layout::{
        Item, Layout, LinkedChunk, Metadata, encode, layout, partition, structural_layout,
    },
};

#[derive(Default)]
pub(super) struct State {
    pub model: Model,
    pub set_id: Option<i64>,
    pub authorized_generation: u64,
    pub(super) cache_instance: Option<u64>,
    layout: Layout,
    pub counters: std::sync::Arc<Counters>,
    pub encoded_bytes: usize,
    pub(super) entry_bytes: BTreeMap<[u8; 16], usize>,
    pub changed: Vec<[u8; 16]>,
    pub committed_from: u64,
    pub committed_revision: u64,
    pub(super) physical_deletes: Vec<crate::account_cache::history::HistoryKey>,
    pub(super) retention_version: u64,
    pub(super) last_write_rows: usize,
}

impl State {
    pub fn with_counters(counters: std::sync::Arc<Counters>) -> Self {
        Self {
            counters,
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        *self = Self::with_counters(std::sync::Arc::clone(&self.counters));
    }

    pub async fn ensure(
        &mut self,
        cache: &AccountCache,
        channel: [u8; 16],
        generation: Generation,
    ) -> Result<(), HistoryError> {
        let instance = cache.history_instance();
        let retention_version = cache.retention_version(channel);
        if self.set_id.is_some()
            && self.cache_instance == Some(instance)
            && self.authorized_generation == generation.get()
            && self.retention_version == retention_version
        {
            return Ok(());
        }
        let set_id = cache.history_authorize(channel, generation, false).await?;
        if self.set_id != Some(set_id)
            || self.cache_instance != Some(instance)
            || self.authorized_generation != generation.get()
            || self.retention_version != retention_version
        {
            let data = cache
                .history_load(channel, generation)
                .await?
                .ok_or(CacheError::InvalidData)?;
            let counters = std::sync::Arc::clone(&self.counters);
            *self = decode(data)?;
            self.counters = counters;
            self.counters
                .cache_loads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.set_id = Some(set_id);
        self.cache_instance = Some(instance);
        self.authorized_generation = generation.get();
        if cache.retention_version(channel) != retention_version {
            self.reset();
            return Err(CacheError::Superseded.into());
        }
        self.retention_version = retention_version;
        Ok(())
    }

    /// One page is one writer transaction. Markers mask obsolete physical rows instead
    /// of issuing an unbounded range delete; retention/revocation reclaim those rows.
    pub async fn persist(
        &mut self,
        cache: &AccountCache,
        channel: [u8; 16],
        epoch: &crate::generation::Epoch,
    ) -> Result<(), HistoryError> {
        let generation = epoch.generation;
        let next_layout = structural_layout(&self.model)?;
        let mut upserts = Vec::new();
        for ((table, id), payload) in &next_layout {
            if self.layout.get(&(*table, *id)) != Some(payload) {
                upserts.push(HistoryRow {
                    table: partition(*table)?,
                    id: *id,
                    payload: payload.clone(),
                });
            }
        }
        let mut sizes = Vec::new();
        for id in &self.model.dirty {
            let entry = self.model.entries.get(id).ok_or(CacheError::InvalidData)?;
            let payload = encode(entry)?;
            sizes.push((*id, payload.len()));
            if !entry.deleted {
                if entry.timeline {
                    upserts.push(HistoryRow {
                        table: HistoryTable::Items,
                        id: *id,
                        payload: encode(&Item::Message {
                            point: entry.point.clone().ok_or(CacheError::InvalidData)?,
                        })?,
                    });
                }
                if entry.pinned {
                    upserts.push(HistoryRow {
                        table: HistoryTable::Pins,
                        id: *id,
                        payload: encode(&true)?,
                    });
                }
                upserts.push(HistoryRow {
                    table: HistoryTable::Messages,
                    id: *id,
                    payload,
                });
            }
        }
        let metadata = encode(&Metadata {
            revision: self.model.revision,
            empty_generation: self.model.empty_generation,
            active_chunks: self.model.chunks.iter().map(|chunk| chunk.id).collect(),
        })?;
        let set_id = self.set_id.ok_or(CacheError::InvalidData)?;
        let row_count = upserts.len();
        let submitted_rows = row_count + self.physical_deletes.len() + 1;
        let written_bytes =
            metadata.len() + upserts.iter().map(|row| row.payload.len()).sum::<usize>();
        if generation.get() == 0 {
            cache
                .retention_commit(
                    channel,
                    set_id,
                    self.committed_revision,
                    metadata,
                    upserts,
                    self.physical_deletes.clone(),
                )
                .await?;
        } else {
            cache
                .history_commit_versioned(
                    channel,
                    set_id,
                    (generation, epoch.cancel.clone()),
                    metadata,
                    upserts,
                    self.physical_deletes.clone(),
                    Some((self.retention_version, self.committed_revision)),
                )
                .await?;
        }
        self.physical_deletes.clear();
        self.last_write_rows = submitted_rows;
        self.counters
            .persisted_rows
            .fetch_add(row_count as u64, std::sync::atomic::Ordering::Relaxed);
        self.counters
            .persisted_bytes
            .fetch_add(written_bytes as u64, std::sync::atomic::Ordering::Relaxed);
        for (id, bytes) in sizes {
            self.encoded_bytes = self
                .encoded_bytes
                .saturating_sub(self.entry_bytes.insert(id, bytes).unwrap_or(0))
                .saturating_add(bytes);
        }
        self.committed_from = self.committed_revision;
        self.committed_revision = self.model.revision;
        self.changed = std::mem::take(&mut self.model.dirty).into_iter().collect();
        self.layout = next_layout;
        Ok(())
    }
}

pub(super) fn decode(data: HistoryData) -> Result<State, HistoryError> {
    let mut state = State {
        set_id: Some(data.set_id),
        ..State::default()
    };
    let metadata = if data.metadata.is_empty() {
        if !data.rows.is_empty() {
            return Err(CacheError::InvalidData.into());
        }
        Metadata {
            revision: 0,
            empty_generation: 0,
            active_chunks: Vec::new(),
        }
    } else {
        serde_json::from_slice::<Metadata>(&data.metadata).map_err(|_| CacheError::InvalidData)?
    };
    state.model.revision = metadata.revision;
    state.committed_revision = metadata.revision;
    state.model.empty_generation = metadata.empty_generation;
    let active_chunks: BTreeSet<_> = metadata.active_chunks.iter().copied().collect();
    if active_chunks.len() != metadata.active_chunks.len() {
        return Err(CacheError::InvalidData.into());
    }
    let mut stored_layout = BTreeMap::new();
    for row in data.rows {
        if row.table == HistoryTable::Messages {
            let entry: Entry =
                serde_json::from_slice(&row.payload).map_err(|_| CacheError::InvalidData)?;
            if entry.revision > state.model.revision {
                return Err(CacheError::InvalidData.into());
            }
            state.entry_bytes.insert(row.id, row.payload.len());
            state.encoded_bytes = state.encoded_bytes.saturating_add(row.payload.len());
            state.model.entries.insert(row.id, entry);
        } else {
            let index = match row.table {
                HistoryTable::Chunks => {
                    if !active_chunks.contains(&row.id) {
                        continue;
                    }
                    let linked: LinkedChunk = serde_json::from_slice(&row.payload)
                        .map_err(|_| CacheError::InvalidData)?;
                    if linked.chunk.id != row.id
                        || linked.chunk.first.key()? > linked.chunk.last.key()?
                    {
                        return Err(CacheError::InvalidData.into());
                    }
                    state.model.chunks.push(linked.chunk);
                    0
                }
                HistoryTable::Items => {
                    let item: Item = serde_json::from_slice(&row.payload)
                        .map_err(|_| CacheError::InvalidData)?;
                    if let Item::Tombstones { ids } = item {
                        state.model.deletions.insert(row.id, ids);
                    }
                    1
                }
                HistoryTable::Pins => 2,
                HistoryTable::Messages => return Err(CacheError::InvalidData.into()),
            };
            stored_layout.insert((index, row.id), row.payload);
        }
    }
    if state.model.chunks.len() != active_chunks.len() {
        return Err(CacheError::InvalidData.into());
    }
    restore(state, &metadata, &stored_layout)
}

fn restore(
    mut state: State,
    metadata: &Metadata,
    stored_layout: &Layout,
) -> Result<State, HistoryError> {
    let tombstones: BTreeSet<_> = state.model.deletions.values().flatten().copied().collect();
    for id in tombstones {
        let entry = state
            .model
            .entries
            .entry(id)
            .or_insert_with(|| Entry::empty(metadata.revision));
        entry.message = None;
        entry.deleted = true;
        entry.timeline = false;
        entry.pinned = false;
        entry.edit = None;
        entry.pending_embeds = None;
        entry.pending_reactions.clear();
    }
    state
        .model
        .chunks
        .sort_by_key(|chunk| chunk.first.key().ok());
    for pair in state.model.chunks.windows(2) {
        if pair[0].last.key()? >= pair[1].first.key()? {
            return Err(CacheError::InvalidData.into());
        }
    }
    state.model.rebuild_order()?;
    let expected = layout(&state.model)?;
    // Markers in the same page commit hide obsolete rows even before physical reclamation.
    if expected
        .iter()
        .any(|(key, payload)| stored_layout.get(key) != Some(payload))
    {
        return Err(CacheError::InvalidData.into());
    }
    state.layout = structural_layout(&state.model)?;
    // Connection epochs are process-local, so persisted numeric equality is never freshness evidence.
    state.model.empty_generation = 0;
    for entry in state.model.entries.values_mut() {
        entry.validated_generation = 0;
    }
    for chunk in &mut state.model.chunks {
        chunk.generation = 0;
        chunk.live_generation = 0;
    }
    Ok(state)
}
