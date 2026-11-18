//! Canonical channel messages and proven ranges; network admission and persistence live separately.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::{Uuid, wire::Message};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    HistoryError,
    chunk_merge::{LowerBound, UpperBound, upper_reaches_lower},
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
pub(super) struct EmptyAnchor {
    pub point: Point,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Chunk {
    pub id: [u8; 16],
    pub first: Point,
    pub last: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_anchor: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_anchor: Option<Point>,
    pub oldest_complete: bool,
    pub newest_complete: bool,
    pub generation: u64,
    pub live_generation: u64,
}

impl Chunk {
    pub fn contains(&self, key: Key) -> Result<bool, HistoryError> {
        Ok(self.first.key()? <= key && key <= self.last.key()?)
    }

    pub fn lower_bound(&self) -> Result<LowerBound, HistoryError> {
        if self.oldest_complete {
            Ok(LowerBound::Unbounded)
        } else if let Some(anchor) = &self.oldest_anchor {
            Ok(LowerBound::Excluded(anchor.key()?))
        } else {
            Ok(LowerBound::Included(self.first.key()?))
        }
    }

    pub fn upper_bound_in(&self, live_generation: Option<u64>) -> Result<UpperBound, HistoryError> {
        let live_tail = self.newest_complete
            && match live_generation {
                Some(gen_id) => gen_id != 0 && self.live_generation == gen_id,
                None => self.live_generation != 0,
            };
        if live_tail {
            Ok(UpperBound::Unbounded)
        } else if let Some(anchor) = &self.newest_anchor {
            Ok(UpperBound::Excluded(anchor.key()?))
        } else {
            Ok(UpperBound::Included(self.last.key()?))
        }
    }

    pub fn overlaps_entries(&self, other: &Self) -> Result<bool, HistoryError> {
        Ok(self.first.key()? <= other.last.key()? && other.first.key()? <= self.last.key()?)
    }

    pub fn touches_or_overlaps_in(
        &self,
        other: &Self,
        live_generation: Option<u64>,
    ) -> Result<bool, HistoryError> {
        Ok(
            upper_reaches_lower(self.upper_bound_in(live_generation)?, other.lower_bound()?)
                && upper_reaches_lower(other.upper_bound_in(live_generation)?, self.lower_bound()?),
        )
    }

    pub fn touches_or_overlaps(&self, other: &Self) -> Result<bool, HistoryError> {
        let same_live = (self.live_generation != 0
            && self.live_generation == other.live_generation)
            .then_some(self.live_generation);
        // Never allow a stale `newest_complete` tail from an earlier epoch to bridge a gap.
        let epoch = same_live.unwrap_or(0);
        self.touches_or_overlaps_in(other, Some(epoch))
    }

    pub fn validate_invariants(&self) -> Result<(), HistoryError> {
        let first = self.first.key()?;
        let last = self.last.key()?;
        if first > last {
            return Err(HistoryError::InvalidPage);
        }
        if self.oldest_complete && self.oldest_anchor.is_some() {
            return Err(HistoryError::InvalidPage);
        }
        if self.newest_complete && self.newest_anchor.is_some() {
            return Err(HistoryError::InvalidPage);
        }
        if let Some(anchor) = &self.oldest_anchor
            && anchor.key()? >= first
        {
            return Err(HistoryError::InvalidPage);
        }
        if let Some(anchor) = &self.newest_anchor
            && anchor.key()? <= last
        {
            return Err(HistoryError::InvalidPage);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Model {
    pub entries: BTreeMap<[u8; 16], Entry>,
    pub order: BTreeMap<Key, [u8; 16]>,
    pub chunks: Vec<Chunk>,
    pub revision: u64,
    pub empty_generation: u64,
    pub empty_before: Option<EmptyAnchor>,
    pub empty_after: Option<EmptyAnchor>,
    pub server_live_generation: u64,
    pub live_arrival_generation: u64,
    pub live_arrival_floor: Option<Point>,
    pub dirty: BTreeSet<[u8; 16]>,
    pub deletions: BTreeMap<[u8; 16], Vec<[u8; 16]>>,
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
                if key.id_bytes() != *id || self.order.insert(key, *id).is_some() {
                    return Err(HistoryError::InvalidPage);
                }
            }
        }
        Ok(())
    }

    pub fn id(value: &str) -> Result<[u8; 16], HistoryError> {
        Uuid::parse_str(value)
            .map(|id| *id.as_bytes())
            .map_err(|_| HistoryError::InvalidPage)
    }
}
