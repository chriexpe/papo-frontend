//! Encoded history coverage/message layout and its version; transaction admission and model
//! restoration live in persistence.
//!
//! Building rows is where an in-memory model meets storage keys, so the layout invariants are
//! checked here, before any row exists. A violation is `HistoryError::Layout` (an SDK defect,
//! nothing written), never `CacheError::InvalidData`, which means stored bytes are corrupt.

use super::{
    HistoryError,
    chunk_id::ChunkId,
    model::{Chunk, Model},
    ordering::Point,
};
use crate::{CacheError, account_cache::history::HistoryTable};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Layout = BTreeMap<(u8, [u8; 16]), Vec<u8>>;

/// Version of the layout written by this build. Version 1 carried no `layout` field and
/// derived chunk identities from message IDs, which could collide after retention; it is
/// never decoded, its channel's history set is rebuilt instead.
pub(super) const LAYOUT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub(super) struct Metadata {
    pub layout: u32,
    pub revision: u64,
    pub empty_generation: u64,
    /// The last chunk identity issued in this history set (see `chunk_id`).
    pub chunk_high_water: u64,
    pub active_chunks: Vec<ChunkId>,
}

/// The layout version a stored metadata blob was written with, read without decoding the
/// rest, so an older layout is recognised as such rather than as corrupt data.
pub(super) fn stored_version(metadata: &[u8]) -> Result<u32, HistoryError> {
    #[derive(Deserialize)]
    struct Stored {
        layout: Option<u32>,
    }
    let stored: Stored = serde_json::from_slice(metadata).map_err(|_| CacheError::InvalidData)?;
    Ok(stored.layout.unwrap_or(1))
}

#[derive(Serialize, Deserialize)]
pub(super) struct LinkedChunk {
    pub chunk: Chunk,
    pub older: Option<ChunkId>,
    pub newer: Option<ChunkId>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(super) enum Item {
    Message { point: Point },
    Gap { cursor: Point, ascending: bool },
    Tombstones { ids: Vec<[u8; 16]> },
}

/// Two runs with one identity would share a chunk row (one silently overwriting the other)
/// and their gap rows, so a model holding them is rejected before any row is built.
fn distinct_identities(chunks: &[Chunk]) -> Result<(), HistoryError> {
    let mut seen = BTreeSet::new();
    for chunk in chunks {
        if !seen.insert(chunk.id) {
            return Err(HistoryError::Layout);
        }
    }
    Ok(())
}

pub(super) fn structural_layout(model: &Model) -> Result<Layout, HistoryError> {
    distinct_identities(&model.chunks)?;
    let mut rows = BTreeMap::new();
    for (index, chunk) in model.chunks.iter().enumerate() {
        let linked = LinkedChunk {
            chunk: chunk.clone(),
            older: index
                .checked_sub(1)
                .and_then(|index| model.chunks.get(index))
                .map(|chunk| chunk.id),
            newer: model.chunks.get(index + 1).map(|chunk| chunk.id),
        };
        rows.insert((0, chunk.id.row_key()), encode(&linked)?);
        for (ascending, complete, point) in [
            (false, chunk.oldest_complete, &chunk.first),
            (true, chunk.newest_complete, &chunk.last),
        ] {
            if !complete {
                let mut hash = Sha256::new();
                hash.update(b"papo-history-gap/v1");
                hash.update(chunk.id.row_key());
                hash.update([u8::from(ascending)]);
                let digest = hash.finalize();
                let mut id = [0; 16];
                id.copy_from_slice(&digest[..16]);
                if rows
                    .insert(
                        (1, id),
                        encode(&Item::Gap {
                            cursor: point.clone(),
                            ascending,
                        })?,
                    )
                    .is_some()
                {
                    return Err(HistoryError::Layout);
                }
            }
        }
    }
    for (id, ids) in &model.deletions {
        if rows
            .insert((1, *id), encode(&Item::Tombstones { ids: ids.clone() })?)
            .is_some()
        {
            return Err(HistoryError::Layout);
        }
    }
    Ok(rows)
}

pub(super) fn layout(model: &Model) -> Result<Layout, HistoryError> {
    let mut rows = structural_layout(model)?;
    for (id, entry) in &model.entries {
        if entry.timeline && !entry.deleted {
            let point = entry.point.as_ref().ok_or(HistoryError::Layout)?;
            if rows
                .insert(
                    (1, *id),
                    encode(&Item::Message {
                        point: point.clone(),
                    })?,
                )
                .is_some()
            {
                return Err(HistoryError::Layout);
            }
        }
        if entry.pinned && !entry.deleted {
            rows.insert((2, *id), encode(&true)?);
        }
    }
    Ok(rows)
}

pub(super) fn partition(index: u8) -> Result<HistoryTable, HistoryError> {
    match index {
        0 => Ok(HistoryTable::Chunks),
        1 => Ok(HistoryTable::Items),
        2 => Ok(HistoryTable::Pins),
        _ => Err(CacheError::InvalidData.into()),
    }
}

pub(super) fn encode(value: &impl Serialize) -> Result<Vec<u8>, HistoryError> {
    serde_json::to_vec(value).map_err(|_| CacheError::InvalidData.into())
}
