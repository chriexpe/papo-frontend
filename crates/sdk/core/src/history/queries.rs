//! Chronological queries over published `HistorySnapshot` coverage and messages.
//! Snapshot publication lives in `publication`; raw channel subscriptions live in `subscription`.

use std::ops::Range;

use papo_api::{HistoryCursor, Uuid};

use crate::{ChronologicalKey, Generation};

use super::{HistoryError, HistorySnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryEmptyAnchor {
    pub key: ChronologicalKey,
    pub generation: Option<Generation>,
}

/// Chronological index window and coverage proof over a `HistorySnapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRange {
    /// Indices in `HistorySnapshot::messages` strictly inside `(after, before)`.
    pub indices: Range<usize>,
    /// `true` only when accepted coverage proves no unknown message lies strictly between `(after, before)`.
    pub complete: bool,
}

/// Result of resolving a chronological cursor against a `HistorySnapshot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPoint {
    /// Resolved microsecond key, upgraded from a legacy millisecond mark when matched in cache.
    pub key: ChronologicalKey,
    /// Index in `HistorySnapshot::messages` when the message is currently held.
    pub index: Option<usize>,
    /// `true` when the message UUID is in `HistorySnapshot::tombstones`.
    pub deleted: bool,
}

impl HistorySnapshot {
    /// Resolves `point` (including legacy millisecond-truncated server marks within ±1 ms)
    /// against this snapshot. Deleted anchors keep their chronological order and never reopen content.
    ///
    /// # Errors
    /// Returns `HistoryError::InvalidPage` if a compared message lacks valid chronological metadata.
    pub fn resolve(&self, point: ChronologicalKey) -> Result<ResolvedPoint, HistoryError> {
        let deleted = self.tombstones.contains(&point.message_id);
        let idx = self.key_lower_bound(point)?;
        if idx < self.messages.len() {
            let candidate = self.key_at(idx)?;
            if candidate == point {
                return Ok(ResolvedPoint {
                    key: candidate,
                    index: (!deleted).then_some(idx),
                    deleted,
                });
            }
        }
        // Legacy server read marks carry millisecond precision; search only that 2 ms window
        // and never reopen deleted anchors.
        if !deleted && point.created_at_us % 1_000 == 0 {
            let low_key = ChronologicalKey {
                created_at_us: point.created_at_us.saturating_sub(1_000),
                message_id: Uuid::nil(),
            };
            let high_us = point.created_at_us.saturating_add(1_000);
            let mut cursor = self.key_lower_bound(low_key)?;
            while cursor < self.messages.len() {
                let candidate = self.key_at(cursor)?;
                if candidate.created_at_us > high_us {
                    break;
                }
                if candidate.message_id == point.message_id {
                    return Ok(ResolvedPoint {
                        key: candidate,
                        index: Some(cursor),
                        deleted: false,
                    });
                }
                cursor += 1;
            }
        }
        Ok(ResolvedPoint {
            key: point,
            index: None,
            deleted,
        })
    }

    /// Returns the index range over `messages` strictly inside `(after, before)` (open when `None`)
    /// together with `complete: bool` indicating whether accepted coverage proves no unknown message
    /// lies strictly between the bounds.
    ///
    /// # Errors
    /// Returns `HistoryError::InvalidPage` if any compared cursor is malformed.
    pub fn range(
        &self,
        after: Option<ChronologicalKey>,
        before: Option<ChronologicalKey>,
    ) -> Result<HistoryRange, HistoryError> {
        let start = match after {
            Some(after) => self.first_after(after)?,
            None => 0,
        };
        let end = match before {
            Some(before) => self.key_lower_bound(before)?,
            None => self.messages.len(),
        };
        let indices = start..end.max(start);
        let complete = self.range_complete(after, before)?;
        Ok(HistoryRange { indices, complete })
    }

    /// Returns the canonical wire `HistoryCursor` for `point` if retained in this snapshot.
    ///
    /// # Errors
    /// Returns `HistoryError::InvalidPage` if a compared message lacks valid chronological metadata.
    pub fn message_cursor_at(
        &self,
        point: ChronologicalKey,
    ) -> Result<Option<HistoryCursor>, HistoryError> {
        let resolved = self.resolve(point)?;
        Ok(resolved
            .index
            .and_then(|idx| self.messages.get(idx)?.cursor()))
    }

    fn key_at(&self, index: usize) -> Result<ChronologicalKey, HistoryError> {
        if let Some(keys) = &self.keys
            && keys.len() == self.messages.len()
        {
            return Ok(keys[index]);
        }
        Ok(ChronologicalKey::from_message(&self.messages[index])?)
    }

    fn partition_point(
        &self,
        mut pred: impl FnMut(ChronologicalKey) -> bool,
    ) -> Result<usize, HistoryError> {
        let mut low = 0;
        let mut high = self.messages.len();
        while low < high {
            let mid = low + (high - low) / 2;
            let key = self.key_at(mid)?;
            if pred(key) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Ok(low)
    }

    fn key_lower_bound(&self, target: ChronologicalKey) -> Result<usize, HistoryError> {
        self.partition_point(|key| key < target)
    }

    fn first_after(&self, target: ChronologicalKey) -> Result<usize, HistoryError> {
        self.partition_point(|key| key <= target)
    }

    fn range_complete(
        &self,
        after: Option<ChronologicalKey>,
        before: Option<ChronologicalKey>,
    ) -> Result<bool, HistoryError> {
        if let (Some(a), Some(b)) = (after, before)
            && a >= b
        {
            return Ok(true);
        }
        if self.empty_complete {
            return Ok(true);
        }
        if let Some(empty_before) = &self.empty_before
            && empty_before.generation.is_some()
            && before.is_some_and(|b| b <= empty_before.key)
            && self.chunks.first().is_none_or(|c| {
                ChronologicalKey::from_cursor(&c.first).is_ok_and(|first| first >= empty_before.key)
            })
        {
            return Ok(true);
        }
        if let Some(empty_after) = &self.empty_after
            && empty_after.generation == Some(self.generation)
            && after.is_some_and(|a| a >= empty_after.key)
            && self.chunks.last().is_none_or(|c| {
                ChronologicalKey::from_cursor(&c.last).is_ok_and(|last| last <= empty_after.key)
            })
        {
            return Ok(true);
        }
        if let (Some(eb), Some(ea)) = (&self.empty_before, &self.empty_after)
            && eb.generation.is_some()
            && ea.generation == Some(self.generation)
            && ea.key < eb.key
            && self.chunks.is_empty()
        {
            return Ok(true);
        }
        for chunk in &self.chunks {
            let first = ChronologicalKey::from_cursor(&chunk.first)?;
            let last = ChronologicalKey::from_cursor(&chunk.last)?;
            let live_tail_proven =
                chunk.newest_complete && chunk.live_generation == Some(self.generation);
            if after.is_some_and(|a| a >= last) && live_tail_proven {
                return Ok(true);
            }
            if chunk.generation.is_none() {
                continue;
            }
            let covers_left = match after {
                None => chunk.oldest_complete,
                Some(a) => {
                    if chunk.oldest_complete {
                        true
                    } else if let Some(anchor) = &chunk.oldest_anchor {
                        a >= ChronologicalKey::from_cursor(anchor)?
                    } else {
                        a >= first
                    }
                }
            };
            if !covers_left {
                continue;
            }
            let covers_right = match before {
                None => live_tail_proven,
                Some(b) => {
                    if b <= last {
                        true
                    } else if let Some(anchor) = &chunk.newest_anchor {
                        b <= ChronologicalKey::from_cursor(anchor)?
                    } else {
                        live_tail_proven
                    }
                }
            };
            if covers_right {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
