//! Chronological snapshot indexing, binary-search lookups and `Asc` page continuity proofs.
//! Domain model lives in `catch_up`; viewport span extraction and coalescing live in `catch_up_spans`.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::{HistoryCursor, Uuid, wire::Message};

use crate::{
    HistorySnapshot, ReadPoint, ReadingError, catch_up::ReadSpan, catch_up_spans::span_contains,
};

/// Proven successor `(after_anchor -> first_returned)` returned by an `Asc` page query.
pub(crate) type AscSuccessors = BTreeMap<ReadPoint, Option<ReadPoint>>;

pub(crate) struct ProvenChunk {
    pub first: ReadPoint,
    pub last: ReadPoint,
    pub oldest_complete: bool,
}

/// Borrowed chronological view of a `HistorySnapshot` using `O(log N)` binary searches instead of full copies.
pub(crate) struct SnapshotIndex<'a> {
    pub snapshot: &'a HistorySnapshot,
    pub self_user: Uuid,
    pub chunks: Vec<ProvenChunk>,
    pub asc_successors: &'a AscSuccessors,
}

impl<'a> SnapshotIndex<'a> {
    pub fn build(
        snapshot: &'a HistorySnapshot,
        self_user: Uuid,
        asc_successors: &'a AscSuccessors,
    ) -> Result<Self, ReadingError> {
        let mut chunks = Vec::with_capacity(snapshot.chunks.len());
        for chunk in &snapshot.chunks {
            let first = ReadPoint::parse(Some(&chunk.first.created_at), Some(&chunk.first.id))?;
            let last = ReadPoint::parse(Some(&chunk.last.created_at), Some(&chunk.last.id))?;
            chunks.push(ProvenChunk {
                first,
                last,
                oldest_complete: chunk.oldest_complete,
            });
        }
        Ok(Self {
            snapshot,
            self_user,
            chunks,
            asc_successors,
        })
    }

    pub fn resolve(&self, point: ReadPoint) -> Result<ReadPoint, ReadingError> {
        resolve_point_in_snapshot(self.snapshot, point)
    }

    pub fn exact_index(&self, point: ReadPoint) -> Result<Option<usize>, ReadingError> {
        let idx = lower_bound_ge(&self.snapshot.messages, point)?;
        match self.snapshot.messages.get(idx) {
            Some(message) if ReadPoint::message(message)? == point => Ok(Some(idx)),
            _ => Ok(None),
        }
    }

    /// True when any retained message sits strictly in `(a, b)`.
    pub fn has_entry_between(&self, a: ReadPoint, b: ReadPoint) -> Result<bool, ReadingError> {
        if a >= b {
            return Ok(false);
        }
        let next_idx = first_after(&self.snapshot.messages, a)?;
        Ok(match self.snapshot.messages.get(next_idx) {
            Some(message) => ReadPoint::message(message)? < b,
            None => false,
        })
    }

    /// True when the snapshot and `Asc` page proofs guarantee zero unseen messages strictly in `(a, b)`.
    pub fn is_contiguous(&self, a: Option<ReadPoint>, b: ReadPoint) -> Result<bool, ReadingError> {
        if let Some(a) = a
            && a >= b
        {
            return Ok(true);
        }
        if self.snapshot.empty_complete {
            return Ok(true);
        }
        if let Some(a) = a
            && self.proven_asc_reaches(a, b)?
        {
            return Ok(true);
        }
        for chunk in &self.chunks {
            if b > chunk.last {
                continue;
            }
            let reaches = match a {
                None => chunk.oldest_complete,
                Some(a) => {
                    chunk.first <= a
                        || (chunk.oldest_complete && b <= chunk.last)
                        || self.proven_asc_reaches(a, chunk.first)?
                }
            };
            if reaches {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn proven_asc_reaches(
        &self,
        start: ReadPoint,
        target: ReadPoint,
    ) -> Result<bool, ReadingError> {
        let mut cursor = start;
        for _ in 0..=self.asc_successors.len() {
            let next = self.asc_successors.get(&cursor).copied().or_else(|| {
                self.asc_successors
                    .iter()
                    .find(|(k, _)| k.message_id == cursor.message_id)
                    .map(|(_, v)| *v)
            });
            let Some(next) = next else {
                return Ok(false);
            };
            match next {
                None => {
                    // An empty `Asc(cursor)` response only proves zero messages existed > cursor
                    // while no loaded entry now sits between `cursor` and `target`.
                    let next_idx = first_after(&self.snapshot.messages, cursor)?;
                    return Ok(match self.snapshot.messages.get(next_idx) {
                        Some(message) => ReadPoint::message(message)? > target,
                        None => true,
                    });
                }
                Some(q) if q >= target => return Ok(true),
                Some(q) if self.chunks_cover_pair(q, target) => return Ok(true),
                Some(q) if q > cursor => cursor = q,
                Some(_) => return Ok(false),
            }
        }
        Ok(false)
    }

    fn chunks_cover_pair(&self, a: ReadPoint, b: ReadPoint) -> bool {
        self.chunks
            .iter()
            .any(|chunk| chunk.first <= a && b <= chunk.last)
    }

    /// Returns the first retained foreign message strictly in `(a, b)` outside `covered_spans`.
    pub fn first_unseen_foreign(
        &self,
        a: Option<ReadPoint>,
        b: ReadPoint,
        covered_spans: &[ReadSpan],
    ) -> Result<Option<(ReadPoint, Option<HistoryCursor>)>, ReadingError> {
        if a.is_some_and(|a| a >= b) {
            return Ok(None);
        }
        let mut idx = match a {
            Some(a) => first_after(&self.snapshot.messages, a)?,
            None => 0,
        };
        let end_idx = lower_bound_ge(&self.snapshot.messages, b)?;
        debug_assert!(covered_spans.windows(2).all(|w| w[0].end < w[1].start));
        while idx < end_idx {
            let message = &self.snapshot.messages[idx];
            let point = ReadPoint::message(message)?;
            let span_idx = covered_spans.partition_point(|span| span.end < point);
            if let Some(&span) = covered_spans.get(span_idx)
                && span_contains(span, point)
            {
                let jumped = first_after(&self.snapshot.messages, span.end)?;
                idx = jumped.max(idx + 1);
                continue;
            }
            if is_foreign_message(message, self.self_user) {
                return Ok(Some((point, message.cursor())));
            }
            idx += 1;
        }
        Ok(None)
    }

    /// True when `(a, b)` is proven contiguous AND contains no foreign message outside `covered_spans`.
    pub fn no_unseen_foreign(
        &self,
        a: Option<ReadPoint>,
        b: ReadPoint,
        covered_spans: &[ReadSpan],
    ) -> Result<bool, ReadingError> {
        if let Some(a) = a
            && a >= b
        {
            return Ok(true);
        }
        if !self.is_contiguous(a, b)? {
            return Ok(false);
        }
        Ok(self.first_unseen_foreign(a, b, covered_spans)?.is_none())
    }
}

fn partition_point_messages(
    messages: &imbl::Vector<Arc<Message>>,
    mut pred: impl FnMut(ReadPoint) -> bool,
) -> Result<usize, ReadingError> {
    let mut low = 0;
    let mut high = messages.len();
    while low < high {
        let mid = low + (high - low) / 2;
        let point = ReadPoint::message(&messages[mid])?;
        if pred(point) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    Ok(low)
}

pub(crate) fn lower_bound_ge(
    messages: &imbl::Vector<Arc<Message>>,
    target: ReadPoint,
) -> Result<usize, ReadingError> {
    partition_point_messages(messages, |point| point < target)
}

pub(crate) fn first_after(
    messages: &imbl::Vector<Arc<Message>>,
    target: ReadPoint,
) -> Result<usize, ReadingError> {
    partition_point_messages(messages, |point| point <= target)
}

pub(crate) fn is_foreign_message(message: &Message, self_user: Uuid) -> bool {
    message
        .author_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        != Some(self_user)
}

pub(crate) fn resolve_point_in_snapshot(
    snapshot: &HistorySnapshot,
    point: ReadPoint,
) -> Result<ReadPoint, ReadingError> {
    let idx = lower_bound_ge(&snapshot.messages, point)?;
    if let Some(message) = snapshot.messages.get(idx) {
        let candidate = ReadPoint::message(message)?;
        if candidate == point {
            return Ok(candidate);
        }
    }
    // Legacy server read marks carry millisecond precision; search only that 2 ms window.
    if !snapshot.tombstones.contains(&point.message_id) && point.created_at_us % 1_000 == 0 {
        let low_key = ReadPoint {
            created_at_us: point.created_at_us.saturating_sub(1_000),
            message_id: Uuid::nil(),
        };
        let high_us = point.created_at_us.saturating_add(1_000);
        let mut cursor = lower_bound_ge(&snapshot.messages, low_key)?;
        while let Some(message) = snapshot.messages.get(cursor) {
            let candidate = ReadPoint::message(message)?;
            if candidate.created_at_us > high_us {
                break;
            }
            if candidate.message_id == point.message_id {
                return Ok(candidate);
            }
            cursor += 1;
        }
    }
    Ok(point)
}
