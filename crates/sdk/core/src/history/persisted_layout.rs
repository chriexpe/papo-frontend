//! Encoded history coverage/message layout; transaction admission and model restoration live in persistence.

use super::{
    HistoryError,
    model::{Chunk, Model},
    ordering::Point,
};
use crate::{CacheError, account_cache::history::HistoryTable};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) type Layout = BTreeMap<(u8, [u8; 16]), Vec<u8>>;

#[derive(Serialize, Deserialize)]
pub(super) struct Metadata {
    pub revision: u64,
    pub empty_generation: u64,
    pub active_chunks: Vec<[u8; 16]>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct LinkedChunk {
    pub chunk: Chunk,
    pub older: Option<[u8; 16]>,
    pub newer: Option<[u8; 16]>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(super) enum Item {
    Message { point: Point },
    Gap { cursor: Point, ascending: bool },
    Tombstones { ids: Vec<[u8; 16]> },
}

pub(super) fn structural_layout(model: &Model) -> Result<Layout, HistoryError> {
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
        rows.insert((0, chunk.id), encode(&linked)?);
        for (ascending, complete, point) in [
            (false, chunk.oldest_complete, &chunk.first),
            (true, chunk.newest_complete, &chunk.last),
        ] {
            if !complete {
                let mut hash = Sha256::new();
                hash.update(b"papo-history-gap/v1");
                hash.update(chunk.id);
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
                    return Err(CacheError::InvalidData.into());
                }
            }
        }
    }
    for (id, ids) in &model.deletions {
        if rows
            .insert((1, *id), encode(&Item::Tombstones { ids: ids.clone() })?)
            .is_some()
        {
            return Err(CacheError::InvalidData.into());
        }
    }
    Ok(rows)
}

pub(super) fn layout(model: &Model) -> Result<Layout, HistoryError> {
    let mut rows = structural_layout(model)?;
    for (id, entry) in &model.entries {
        if entry.timeline && !entry.deleted {
            let point = entry.point.as_ref().ok_or(CacheError::InvalidData)?;
            if rows
                .insert(
                    (1, *id),
                    encode(&Item::Message {
                        point: point.clone(),
                    })?,
                )
                .is_some()
            {
                return Err(CacheError::InvalidData.into());
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
