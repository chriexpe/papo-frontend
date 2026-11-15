//! Chronological snapshot indexing, asc-page continuity proofs and interval span coalescing.
//! Domain model and public catch-up types belong to `catch_up`.

use std::collections::BTreeMap;

use papo_api::{HistoryCursor, Uuid, wire::Message};

use crate::{
    HistorySnapshot, ReadPoint, ReadingError,
    catch_up::{CatchUpInterval, ReadSpan},
};

/// Proven successor `(after_anchor -> first_returned)` returned by an `Asc` page query.
pub(crate) type AscSuccessors = BTreeMap<ReadPoint, Option<ReadPoint>>;

pub(crate) struct SnapshotEntry {
    pub point: ReadPoint,
    pub foreign: bool,
    pub cursor: Option<HistoryCursor>,
}

pub(crate) struct ProvenChunk {
    pub first: ReadPoint,
    pub last: ReadPoint,
    pub oldest_complete: bool,
}

/// Pre-indexed chronological view of a `HistorySnapshot` for deterministic boundary math.
pub(crate) struct SnapshotIndex<'a> {
    pub entries: Vec<SnapshotEntry>,
    pub by_id: BTreeMap<Uuid, ReadPoint>,
    pub chunks: Vec<ProvenChunk>,
    pub empty_complete: bool,
    pub asc_successors: &'a AscSuccessors,
}

impl<'a> SnapshotIndex<'a> {
    pub fn build(
        snapshot: &HistorySnapshot,
        self_user: Uuid,
        asc_successors: &'a AscSuccessors,
    ) -> Result<Self, ReadingError> {
        let mut entries = Vec::with_capacity(snapshot.messages.len());
        let mut by_id = BTreeMap::new();
        for message in &snapshot.messages {
            let point = ReadPoint::message(message)?;
            let foreign = is_foreign_message(message, self_user);
            by_id.insert(point.message_id, point);
            entries.push(SnapshotEntry {
                point,
                foreign,
                cursor: message.cursor(),
            });
        }
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
            entries,
            by_id,
            chunks,
            empty_complete: snapshot.empty_complete,
            asc_successors,
        })
    }

    pub fn resolve(&self, point: ReadPoint) -> ReadPoint {
        self.by_id.get(&point.message_id).copied().unwrap_or(point)
    }

    /// True when the snapshot and `Asc` page proofs guarantee zero unseen messages strictly in `(a, b)`.
    pub fn is_contiguous(&self, a: Option<ReadPoint>, b: ReadPoint) -> bool {
        if let Some(a) = a
            && a >= b
        {
            return true;
        }
        if self.empty_complete {
            return true;
        }
        if let Some(a) = a
            && self.proven_asc_reaches(a, b)
        {
            return true;
        }
        self.chunks.iter().any(|chunk| {
            let upper_ok = b <= chunk.last;
            if !upper_ok {
                return false;
            }
            match a {
                None => chunk.oldest_complete,
                Some(a) => {
                    chunk.first <= a
                        || (chunk.oldest_complete && b <= chunk.last)
                        || self.proven_asc_reaches(a, chunk.first)
                }
            }
        })
    }

    fn proven_asc_reaches(&self, start: ReadPoint, target: ReadPoint) -> bool {
        let mut cursor = start;
        for _ in 0..=self.asc_successors.len() {
            let next = self.asc_successors.get(&cursor).copied().or_else(|| {
                self.asc_successors
                    .iter()
                    .find(|(k, _)| k.message_id == cursor.message_id)
                    .map(|(_, v)| *v)
            });
            let Some(next) = next else {
                return false;
            };
            match next {
                None => {
                    // An empty `Asc(cursor)` response only proves zero messages existed > cursor
                    // while no loaded entry now sits between `cursor` and `target`.
                    let next_idx = self.entries.partition_point(|entry| entry.point <= cursor);
                    return self
                        .entries
                        .get(next_idx)
                        .is_none_or(|entry| entry.point > target);
                }
                Some(q) if q >= target => return true,
                Some(q) if self.chunks_cover_pair(q, target) => return true,
                Some(q) if q > cursor => cursor = q,
                Some(_) => return false,
            }
        }
        false
    }

    fn chunks_cover_pair(&self, a: ReadPoint, b: ReadPoint) -> bool {
        self.chunks
            .iter()
            .any(|chunk| chunk.first <= a && b <= chunk.last)
    }

    /// True when `(a, b)` is proven contiguous AND contains no foreign message outside `covered_spans`.
    pub fn no_unseen_foreign(
        &self,
        a: Option<ReadPoint>,
        b: ReadPoint,
        covered_spans: &[ReadSpan],
    ) -> bool {
        if let Some(a) = a
            && a >= b
        {
            return true;
        }
        if !self.is_contiguous(a, b) {
            return false;
        }
        let start_idx = match a {
            Some(a) => self.entries.partition_point(|entry| entry.point <= a),
            None => 0,
        };
        let end_idx = self.entries.partition_point(|entry| entry.point < b);
        for entry in &self.entries[start_idx..end_idx] {
            if entry.foreign
                && !covered_spans
                    .iter()
                    .any(|span| span_contains(*span, entry.point))
            {
                return false;
            }
        }
        true
    }
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
    let target = point.message_id.to_string();
    for message in &snapshot.messages {
        if message.id.as_deref() == Some(target.as_str()) {
            return ReadPoint::message(message);
        }
    }
    Ok(point)
}

pub(crate) fn span_contains(span: ReadSpan, point: ReadPoint) -> bool {
    span.start <= point && point <= span.end
}

pub(crate) fn contiguous_segments(
    index: &SnapshotIndex<'_>,
    sorted: &[ReadPoint],
) -> Vec<ReadSpan> {
    let mut spans = Vec::new();
    let Some(&first) = sorted.first() else {
        return spans;
    };
    let mut seg_start = first;
    let mut seg_end = first;
    for &point in &sorted[1..] {
        if index.is_contiguous(Some(seg_end), point) {
            seg_end = point;
        } else {
            spans.push(ReadSpan::normalized(seg_start, seg_end));
            seg_start = point;
            seg_end = point;
        }
    }
    spans.push(ReadSpan::normalized(seg_start, seg_end));
    spans
}

impl CatchUpInterval {
    pub(crate) fn apply_span(&mut self, index: &SnapshotIndex<'_>, span: ReadSpan) {
        if self
            .start_after
            .is_some_and(|start_after| span.end <= start_after)
            || span.start >= self.end_before
        {
            return;
        }
        self.covered_spans.push(span);
        self.coalesce(index);
    }

    pub(crate) fn coalesce(&mut self, index: &SnapshotIndex<'_>) {
        if !self.covered_spans.is_empty() {
            self.covered_spans
                .sort_by_key(|span| (span.start, span.end));
            let mut merged: Vec<ReadSpan> = Vec::with_capacity(self.covered_spans.len());
            for span in self.covered_spans.drain(..) {
                if let Some(last) = merged.last_mut()
                    && (span.start <= last.end
                        || index.no_unseen_foreign(Some(last.end), span.start, &[]))
                {
                    if span.start < last.start {
                        last.start = span.start;
                    }
                    if span.end > last.end {
                        last.end = span.end;
                    }
                } else {
                    merged.push(span);
                }
            }
            self.covered_spans = merged;
        }

        let mut remaining = Vec::with_capacity(self.covered_spans.len());
        for span in self.covered_spans.drain(..) {
            let touches_start = match self.start_after {
                Some(s) => span.start <= s || index.no_unseen_foreign(Some(s), span.start, &[]),
                None => index.no_unseen_foreign(None, span.start, &[]),
            };
            if touches_start {
                let candidate = span.end.min(self.end_before);
                self.start_after = Some(self.start_after.map_or(candidate, |s| s.max(candidate)));
            } else {
                remaining.push(span);
            }
        }
        self.covered_spans = remaining;

        while let Some(&last) = self.covered_spans.last() {
            let touches_end = last.end >= self.end_before
                || index.no_unseen_foreign(Some(last.end), self.end_before, &[]);
            if !touches_end {
                break;
            }
            self.covered_spans.pop();
            if last.start < self.end_before {
                self.end_before = last.start;
            }
        }

        self.covered_spans.retain(|span| {
            span.end < self.end_before
                && self.start_after.is_none_or(|s| span.end > s)
                && span.start < self.end_before
        });
    }

    pub(crate) fn is_closed(&self, index: &SnapshotIndex<'_>) -> bool {
        self.start_after.is_some_and(|s| s >= self.end_before)
            || index.no_unseen_foreign(self.start_after, self.end_before, &self.covered_spans)
    }
}
