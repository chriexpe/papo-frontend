//! Incremental model rows and atomic tombstone/coverage markers, and the per-channel rebuild of
//! a history set stored in an older layout; cache owns revocation and physical purge.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AccountCache, CacheError, Generation,
    account_cache::history::{HistoryData, HistoryRow, HistoryTable},
};

use super::{
    HistoryError,
    chunk_id::ChunkIds,
    diagnostics::Counters,
    model::{Entry, Model},
    persisted_layout::{
        Item, LAYOUT_VERSION, Layout, LinkedChunk, Metadata, encode, layout, partition,
        stored_version, structural_layout,
    },
};

/// What a stored history set holds for this build.
pub(super) enum Stored {
    /// A set in the current layout, restored.
    Current(Box<State>),
    /// A set written in another layout version. It is rebuildable cache, never decoded and
    /// never reported as corrupt: an open replaces it with an empty set and retention retires
    /// it, for this channel only.
    Outdated,
}

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
        let mut set_id = cache.history_authorize(channel, generation, false).await?;
        if self.set_id != Some(set_id)
            || self.cache_instance != Some(instance)
            || self.authorized_generation != generation.get()
            || self.retention_version != retention_version
        {
            // The set authorized above can be retired before this load (a cold retention
            // pass on an outdated layout). That is a lost race, not corruption: the next
            // attempt authorizes the channel's current set.
            let data = cache
                .history_load(channel, generation)
                .await?
                .ok_or(CacheError::Superseded)?;
            let counters = std::sync::Arc::clone(&self.counters);
            *self = match decode(data)? {
                Stored::Current(state) => *state,
                Stored::Outdated => {
                    // `fresh` revokes the channel's current set (the outdated one, under this
                    // channel's state lock) in the same fenced transaction that allocates an
                    // empty replacement; other channels are untouched.
                    set_id = cache.history_authorize(channel, generation, true).await?;
                    Self::default()
                }
            };
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
            let entry = self.model.entries.get(id).ok_or(HistoryError::Layout)?;
            let payload = encode(entry)?;
            sizes.push((*id, payload.len()));
            if !entry.deleted {
                if entry.timeline {
                    upserts.push(HistoryRow {
                        table: HistoryTable::Items,
                        id: *id,
                        payload: encode(&Item::Message {
                            point: entry.point.clone().ok_or(HistoryError::Layout)?,
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
            layout: LAYOUT_VERSION,
            revision: self.model.revision,
            empty_generation: self.model.empty_generation,
            chunk_high_water: self.model.chunk_ids.high_water(),
            active_chunks: self.model.chunks.iter().map(|chunk| chunk.id).collect(),
        })?;
        let set_id = self.set_id.ok_or(HistoryError::Unverified)?;
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

#[cfg(test)]
impl Stored {
    /// The restored state of a set in the current layout.
    pub fn expect_current(self) -> State {
        match self {
            Self::Current(state) => *state,
            Self::Outdated => panic!("history set stored in an outdated layout"),
        }
    }
}

/// Restores a stored set. Errors mean the stored bytes are inconsistent (`InvalidData`); a
/// set from another layout version is `Stored::Outdated`, checked before anything else.
pub(super) fn decode(data: HistoryData) -> Result<Stored, HistoryError> {
    let mut state = State {
        set_id: Some(data.set_id),
        ..State::default()
    };
    let metadata = if data.metadata.is_empty() {
        if !data.rows.is_empty() {
            return Err(CacheError::InvalidData.into());
        }
        Metadata {
            layout: LAYOUT_VERSION,
            revision: 0,
            empty_generation: 0,
            chunk_high_water: 0,
            active_chunks: Vec::new(),
        }
    } else {
        if stored_version(&data.metadata)? != LAYOUT_VERSION {
            return Ok(Stored::Outdated);
        }
        serde_json::from_slice::<Metadata>(&data.metadata).map_err(|_| CacheError::InvalidData)?
    };
    state.model.revision = metadata.revision;
    state.committed_revision = metadata.revision;
    state.model.empty_generation = metadata.empty_generation;
    state.model.chunk_ids = ChunkIds::resume(metadata.chunk_high_water);
    // Each active identity appears once and was issued before the stored high-water mark,
    // so allocation after reload can never issue it again.
    let mut active_chunks = BTreeMap::new();
    for id in &metadata.active_chunks {
        if !state.model.chunk_ids.issued(*id) || active_chunks.insert(id.row_key(), *id).is_some() {
            return Err(CacheError::InvalidData.into());
        }
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
                    let Some(id) = active_chunks.get(&row.id) else {
                        continue;
                    };
                    let linked: LinkedChunk = serde_json::from_slice(&row.payload)
                        .map_err(|_| CacheError::InvalidData)?;
                    if linked.chunk.id != *id
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
    restore(state, &metadata, &stored_layout).map(|state| Stored::Current(Box::new(state)))
}

/// A layout invariant broken by a model restored from disk means the stored rows disagree.
fn stored_fault(error: HistoryError) -> HistoryError {
    match error {
        HistoryError::Layout => CacheError::InvalidData.into(),
        error => error,
    }
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
    let expected = layout(&state.model).map_err(stored_fault)?;
    // Markers in the same page commit hide obsolete rows even before physical reclamation.
    if expected
        .iter()
        .any(|(key, payload)| stored_layout.get(key) != Some(payload))
    {
        return Err(CacheError::InvalidData.into());
    }
    state.layout = structural_layout(&state.model).map_err(stored_fault)?;
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
