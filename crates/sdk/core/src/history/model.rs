//! Canonical channel messages and proven ranges; network admission and persistence live separately.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound::{Excluded, Included},
};

use papo_api::{HistoryOrder, Uuid, wire::Message};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    HistoryError, HistoryGap,
    ordering::{Key, Point},
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub point: Option<Point>,
    pub message: Option<Message>,
    pub deleted: bool,
    pub timeline: bool,
    pub revision: u64,
    pub validated_generation: u64,
    pub reaction_revision: u64,
    pub embeds_revision: u64,
    pub pin_revision: u64,
    pub pinned: bool,
    pub edit: Option<(String, String)>,
    pub pending_embeds: Option<Vec<papo_api::wire::Embed>>,
    pub pending_reactions: Vec<papo_api::wire::MessageReactionSummary>,
}

impl Entry {
    pub fn empty(revision: u64) -> Self {
        Self {
            point: None,
            message: None,
            deleted: false,
            timeline: false,
            revision,
            validated_generation: 0,
            reaction_revision: 0,
            embeds_revision: 0,
            pin_revision: 0,
            pinned: false,
            edit: None,
            pending_embeds: None,
            pending_reactions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Chunk {
    pub id: [u8; 16],
    pub first: Point,
    pub last: Point,
    pub oldest_complete: bool,
    pub newest_complete: bool,
    pub generation: u64,
    pub live_generation: u64,
}

impl Chunk {
    pub fn contains(&self, key: Key) -> Result<bool, HistoryError> {
        Ok(self.first.key()? <= key && key <= self.last.key()?)
    }
}

#[derive(Default)]
pub(super) struct Model {
    pub entries: BTreeMap<[u8; 16], Entry>,
    pub order: BTreeMap<Key, [u8; 16]>,
    pub chunks: Vec<Chunk>,
    pub revision: u64,
    pub empty_generation: u64,
    pub dirty: BTreeSet<[u8; 16]>,
    pub deletions: BTreeMap<[u8; 16], Vec<[u8; 16]>>,
}

pub(super) struct CachedPage {
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub gap: Option<HistoryGap>,
    pub fresh: bool,
}

impl Model {
    pub fn next_revision(&mut self) -> Result<u64, HistoryError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(HistoryError::InvalidPage)?;
        Ok(self.revision)
    }

    pub fn record_deletions(&mut self, revision: u64, ids: Vec<[u8; 16]>) {
        if ids.is_empty() {
            return;
        }
        let digest = Sha256::digest(
            [
                b"papo-history-tombstones/v1".as_slice(),
                &revision.to_be_bytes(),
            ]
            .concat(),
        );
        let mut id = [0; 16];
        id.copy_from_slice(&digest[..16]);
        self.deletions.insert(id, ids);
    }

    pub fn rebuild_order(&mut self) -> Result<(), HistoryError> {
        self.order.clear();
        for (id, entry) in &self.entries {
            if entry.timeline && !entry.deleted {
                let key = entry
                    .point
                    .as_ref()
                    .ok_or(HistoryError::InvalidPage)?
                    .key()?;
                if key.id != *id || self.order.insert(key, *id).is_some() {
                    return Err(HistoryError::InvalidPage);
                }
            }
        }
        Ok(())
    }

    pub fn page(
        &self,
        cursor: Option<&Point>,
        order: HistoryOrder,
        limit: usize,
        generation: u64,
    ) -> Result<CachedPage, HistoryError> {
        let boundary = cursor.map(Point::key).transpose()?;
        let chunk = match boundary {
            Some(key) => self
                .chunks
                .iter()
                .find(|chunk| chunk.contains(key).unwrap_or(false)),
            None if order == HistoryOrder::Desc => self.chunks.last(),
            None => self.chunks.first(),
        };
        let Some(chunk) = chunk else {
            return Ok(CachedPage {
                messages: Vec::new(),
                has_more: self.empty_generation != generation,
                gap: cursor.map(|cursor| HistoryGap {
                    cursor: cursor.cursor(),
                    order,
                }),
                fresh: self.empty_generation == generation,
            });
        };
        // A cursor in a hole must not jump directly to the next cached run.
        let first = chunk.first.key()?;
        let last = chunk.last.key()?;
        let lower = match (order, boundary) {
            (HistoryOrder::Asc, Some(key)) => Excluded(key),
            _ => Included(first),
        };
        let upper = match (order, boundary) {
            (HistoryOrder::Desc, Some(key)) => Excluded(key),
            _ => Included(last),
        };
        let range = self.order.range((lower, upper));
        let payload = |(_, id): (&Key, &[u8; 16])| self.entries.get(id)?.message.as_ref();
        let candidates: Vec<_> = match order {
            HistoryOrder::Desc => range.rev().filter_map(payload).take(limit + 1).collect(),
            HistoryOrder::Asc => range.filter_map(payload).take(limit + 1).collect(),
        };
        let more_cached = candidates.len() > limit;
        let complete = match order {
            HistoryOrder::Desc => chunk.oldest_complete,
            HistoryOrder::Asc => chunk.newest_complete,
        };
        let gap = (!more_cached && !complete).then(|| HistoryGap {
            cursor: match order {
                HistoryOrder::Desc => chunk.first.cursor(),
                HistoryOrder::Asc => chunk.last.cursor(),
            },
            order,
        });
        let payloads_fresh = candidates.iter().take(limit).all(|message| {
            message
                .id
                .as_deref()
                .and_then(|id| Self::id(id).ok())
                .and_then(|id| self.entries.get(&id))
                .is_some_and(|entry| entry.validated_generation == generation)
        });
        let start_proven = boundary.is_some()
            || match order {
                HistoryOrder::Desc => chunk.newest_complete && chunk.live_generation == generation,
                HistoryOrder::Asc => chunk.oldest_complete && chunk.generation == generation,
            };
        let terminal_proven = more_cached
            || match order {
                HistoryOrder::Desc => chunk.oldest_complete && chunk.generation == generation,
                HistoryOrder::Asc => chunk.newest_complete && chunk.live_generation == generation,
            };
        Ok(CachedPage {
            messages: candidates.into_iter().take(limit).cloned().collect(),
            has_more: more_cached || !complete,
            gap,
            fresh: payloads_fresh && start_proven && terminal_proven,
        })
    }

    pub fn id(value: &str) -> Result<[u8; 16], HistoryError> {
        Uuid::parse_str(value)
            .map(|id| *id.as_bytes())
            .map_err(|_| HistoryError::InvalidPage)
    }
}
