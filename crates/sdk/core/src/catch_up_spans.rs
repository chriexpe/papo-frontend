//! Viewport span extraction, unseen-foreign scanning, and `CatchUpInterval` boundary coalescing.
//! Chronological point resolution and range completeness live in `history::queries`.

use papo_api::{HistoryCursor, Uuid};

use crate::{
    ChronologicalKey, HistorySnapshot, ReadPoint, ReadingError,
    catch_up::{CatchUpInterval, ReadSpan},
};

pub(crate) fn span_contains(span: ReadSpan, point: ReadPoint) -> bool {
    span.start <= point && point <= span.end
}

pub(crate) fn contiguous_segments(
    snapshot: &HistorySnapshot,
    sorted: &[ReadPoint],
) -> Result<Vec<ReadSpan>, ReadingError> {
    let mut spans = Vec::new();
    let Some(&first) = sorted.first() else {
        return Ok(spans);
    };
    let mut seg_start = first;
    let mut seg_end = first;
    for &point in &sorted[1..] {
        if point == seg_end {
            continue;
        }
        // Contiguous history alone does not expose omitted interior rows inside a loaded chunk.
        let adjacent_complete = if point > seg_end {
            let between = snapshot.range(Some(seg_end), Some(point))?;
            between.complete && between.indices.is_empty()
        } else {
            false
        };
        if adjacent_complete {
            seg_end = point;
        } else {
            spans.push(ReadSpan::normalized(seg_start, seg_end));
            seg_start = point;
            seg_end = point;
        }
    }
    spans.push(ReadSpan::normalized(seg_start, seg_end));
    Ok(spans)
}

pub(crate) fn no_unseen_foreign(
    snapshot: &HistorySnapshot,
    self_user: Uuid,
    after: Option<ReadPoint>,
    before: ReadPoint,
    covered_spans: &[ReadSpan],
) -> Result<bool, ReadingError> {
    let range = snapshot.range(after, Some(before))?;
    if !range.complete {
        return Ok(false);
    }
    for idx in range.indices {
        let message = &snapshot.messages[idx];
        if !crate::is_foreign_message(message, self_user) {
            continue;
        }
        let point = ChronologicalKey::from_message(message)?;
        if !covered_spans.iter().any(|&span| span_contains(span, point)) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn first_unseen_foreign(
    snapshot: &HistorySnapshot,
    self_user: Uuid,
    after: Option<ReadPoint>,
    before: ReadPoint,
    covered_spans: &[ReadSpan],
) -> Result<Option<(ReadPoint, HistoryCursor)>, ReadingError> {
    let range = snapshot.range(after, Some(before))?;
    for idx in range.indices {
        let message = &snapshot.messages[idx];
        if !crate::is_foreign_message(message, self_user) {
            continue;
        }
        let point = ChronologicalKey::from_message(message)?;
        if !covered_spans.iter().any(|&span| span_contains(span, point)) {
            return Ok(Some((point, point.to_cursor()?)));
        }
    }
    Ok(None)
}

impl CatchUpInterval {
    pub(crate) fn apply_spans(
        &mut self,
        snapshot: &HistorySnapshot,
        self_user: Uuid,
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
            self.coalesce(snapshot, self_user)?;
        }
        Ok(())
    }

    pub(crate) fn coalesce(
        &mut self,
        snapshot: &HistorySnapshot,
        self_user: Uuid,
    ) -> Result<(), ReadingError> {
        if !self.covered_spans.is_empty() {
            self.covered_spans
                .sort_by_key(|span| (span.start, span.end));
            let mut merged: Vec<ReadSpan> = Vec::with_capacity(self.covered_spans.len());
            for span in self.covered_spans.drain(..) {
                if let Some(last) = merged.last_mut()
                    && (span.start <= last.end
                        || no_unseen_foreign(snapshot, self_user, Some(last.end), span.start, &[])?)
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
                Some(s) => {
                    span.start <= s
                        || no_unseen_foreign(snapshot, self_user, Some(s), span.start, &[])?
                }
                None => no_unseen_foreign(snapshot, self_user, None, span.start, &[])?,
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
                || no_unseen_foreign(snapshot, self_user, Some(last.end), self.end_before, &[])?;
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

    pub(crate) fn is_closed(
        &self,
        snapshot: &HistorySnapshot,
        self_user: Uuid,
    ) -> Result<bool, ReadingError> {
        Ok(self.start_after.is_some_and(|s| s >= self.end_before)
            || no_unseen_foreign(
                snapshot,
                self_user,
                self.start_after,
                self.end_before,
                &self.covered_spans,
            )?)
    }
}
