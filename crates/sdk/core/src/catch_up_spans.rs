//! Viewport span extraction and `CatchUpInterval` boundary coalescing.
//! Snapshot binary search and continuity proofs live in `catch_up_index`.

use crate::{
    ReadPoint, ReadingError,
    catch_up::{CatchUpInterval, ReadSpan},
    catch_up_index::SnapshotIndex,
};

pub(crate) fn span_contains(span: ReadSpan, point: ReadPoint) -> bool {
    span.start <= point && point <= span.end
}

pub(crate) fn contiguous_segments(
    index: &SnapshotIndex<'_>,
    sorted: &[ReadPoint],
) -> Result<Vec<ReadSpan>, ReadingError> {
    let mut spans = Vec::new();
    let Some(&first) = sorted.first() else {
        return Ok(spans);
    };
    let mut seg_start = first;
    let mut seg_end = first;
    let mut seg_end_idx = index.exact_index(first)?;
    for &point in &sorted[1..] {
        let next_idx = match seg_end_idx {
            Some(idx) => {
                let candidate = idx + 1;
                match index.snapshot.messages.get(candidate) {
                    Some(message) if ReadPoint::message(message)? == point => Some(candidate),
                    _ => index.exact_index(point)?,
                }
            }
            None => index.exact_index(point)?,
        };
        // Contiguous history alone does not expose omitted interior rows inside a loaded chunk.
        let no_omitted_rows = match (seg_end_idx, next_idx) {
            (Some(a), Some(b)) => b == a + 1,
            _ => !index.has_entry_between(seg_end, point)?,
        };
        if no_omitted_rows && index.is_contiguous(Some(seg_end), point)? {
            seg_end = point;
        } else {
            spans.push(ReadSpan::normalized(seg_start, seg_end));
            seg_start = point;
            seg_end = point;
        }
        seg_end_idx = next_idx;
    }
    spans.push(ReadSpan::normalized(seg_start, seg_end));
    Ok(spans)
}

impl CatchUpInterval {
    pub(crate) fn apply_spans(
        &mut self,
        index: &SnapshotIndex<'_>,
        spans: &[ReadSpan],
    ) -> Result<(), ReadingError> {
        let mut added = false;
        for &span in spans {
            if self
                .start_after
                .is_some_and(|start_after| span.end <= start_after)
                || span.start >= self.end_before
            {
                continue;
            }
            self.covered_spans.push(span);
            added = true;
        }
        if added {
            self.coalesce(index)?;
        }
        Ok(())
    }

    pub(crate) fn coalesce(&mut self, index: &SnapshotIndex<'_>) -> Result<(), ReadingError> {
        if !self.covered_spans.is_empty() {
            self.covered_spans
                .sort_by_key(|span| (span.start, span.end));
            let mut merged: Vec<ReadSpan> = Vec::with_capacity(self.covered_spans.len());
            for span in self.covered_spans.drain(..) {
                if let Some(last) = merged.last_mut()
                    && (span.start <= last.end
                        || index.no_unseen_foreign(Some(last.end), span.start, &[])?)
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
                Some(s) => span.start <= s || index.no_unseen_foreign(Some(s), span.start, &[])?,
                None => index.no_unseen_foreign(None, span.start, &[])?,
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
                || index.no_unseen_foreign(Some(last.end), self.end_before, &[])?;
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
        Ok(())
    }

    pub(crate) fn is_closed(&self, index: &SnapshotIndex<'_>) -> Result<bool, ReadingError> {
        Ok(self.start_after.is_some_and(|s| s >= self.end_before)
            || index.no_unseen_foreign(self.start_after, self.end_before, &self.covered_spans)?)
    }
}
