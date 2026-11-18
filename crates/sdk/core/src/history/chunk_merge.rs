//! Chunk interval bound comparisons, coalescing, absorption, and snapshot gap projection.
//! Page coverage orchestration lives in `coverage`; empty-page anchors live in `empty_coverage`.

use papo_api::HistoryOrder;

use crate::Generation;

use super::{
    HistoryError, HistoryGap,
    model::{Chunk, EmptyAnchor},
    ordering::{Key, Point},
    queries::HistoryEmptyAnchor,
    subscription::HistoryChunk,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LowerBound {
    Unbounded,
    Included(Key),
    Excluded(Key),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpperBound {
    Excluded(Key),
    Included(Key),
    Unbounded,
}

pub(super) fn upper_reaches_lower(upper: UpperBound, lower: LowerBound) -> bool {
    match (upper, lower) {
        (UpperBound::Unbounded, _) | (_, LowerBound::Unbounded) => true,
        (UpperBound::Included(u) | UpperBound::Excluded(u), LowerBound::Included(l))
        | (UpperBound::Included(u), LowerBound::Excluded(l)) => u >= l,
        (UpperBound::Excluded(u), LowerBound::Excluded(l)) => u > l,
    }
}

pub(super) fn should_coalesce_chunks(left: &Chunk, right: &Chunk) -> Result<bool, HistoryError> {
    if left.overlaps_entries(right)? {
        return Ok(true);
    }
    Ok((left.generation != 0) == (right.generation != 0) && left.touches_or_overlaps(right)?)
}

pub(super) fn seam_proven(left: &Chunk, right: &Chunk) -> bool {
    let Ok(left_last) = left.last.key() else {
        return false;
    };
    let Ok(right_first) = right.first.key() else {
        return false;
    };
    let left_proves = left.generation != 0
        && (left.newest_complete
            || left
                .newest_anchor
                .as_ref()
                .and_then(|anchor| anchor.key().ok())
                .is_some_and(|bk| bk >= right_first));
    let right_proves = right.generation != 0
        && (right.oldest_complete
            || right
                .oldest_anchor
                .as_ref()
                .and_then(|anchor| anchor.key().ok())
                .is_some_and(|ak| ak <= left_last));
    left_proves || right_proves
}

pub(super) fn coalesce_chunks(mut chunks: Vec<Chunk>) -> Result<Vec<Chunk>, HistoryError> {
    chunks.sort_by_key(|chunk| chunk.first.key().ok());
    let mut idx = 0;
    while idx + 1 < chunks.len() {
        if should_coalesce_chunks(&chunks[idx], &chunks[idx + 1])? {
            let right = chunks.remove(idx + 1);
            absorb_chunk(&mut chunks[idx], right, false)?;
        } else {
            idx += 1;
        }
    }
    let last_idx = chunks.len().saturating_sub(1);
    for (index, chunk) in chunks.iter_mut().enumerate() {
        // Only the chronological tail chunk can ever carry a live-edge proof.
        if index < last_idx && chunk.newest_complete {
            chunk.newest_complete = false;
            chunk.live_generation = 0;
        }
    }
    Ok(chunks)
}

pub(super) fn absorb_chunk(
    dst: &mut Chunk,
    src: Chunk,
    refreshed: bool,
) -> Result<(), HistoryError> {
    dst.id = dst.id.min(src.id);
    if src.first.key()? < dst.first.key()? {
        dst.first = src.first;
    }
    if src.last.key()? > dst.last.key()? {
        dst.last = src.last;
    }
    let first_key = dst.first.key()?;
    let last_key = dst.last.key()?;
    dst.oldest_complete |= src.oldest_complete;
    if dst.oldest_complete {
        dst.oldest_anchor = None;
    } else {
        dst.oldest_anchor = [dst.oldest_anchor.take(), src.oldest_anchor]
            .into_iter()
            .flatten()
            .filter_map(|anchor| {
                let ak = anchor.key().ok()?;
                (ak < first_key).then_some((ak, anchor))
            })
            .min_by_key(|(ak, _)| *ak)
            .map(|(_, anchor)| anchor);
    }
    dst.newest_complete |= src.newest_complete;
    if dst.newest_complete {
        dst.newest_anchor = None;
    } else {
        dst.newest_anchor = [dst.newest_anchor.take(), src.newest_anchor]
            .into_iter()
            .flatten()
            .filter_map(|anchor| {
                let bk = anchor.key().ok()?;
                (bk > last_key).then_some((bk, anchor))
            })
            .max_by_key(|(bk, _)| *bk)
            .map(|(_, anchor)| anchor);
    }
    dst.live_generation = if dst.newest_complete {
        dst.live_generation.max(src.live_generation)
    } else {
        0
    };
    if !refreshed {
        dst.generation = dst.generation.min(src.generation);
    }
    Ok(())
}

pub(super) fn lower_covers(page: LowerBound, existing: LowerBound) -> bool {
    match (page, existing) {
        (LowerBound::Unbounded, _) => true,
        (_, LowerBound::Unbounded) => false,
        (LowerBound::Excluded(p), LowerBound::Included(e)) => p < e,
        (LowerBound::Excluded(p), LowerBound::Excluded(e))
        | (LowerBound::Included(p), LowerBound::Included(e) | LowerBound::Excluded(e)) => p <= e,
    }
}

pub(super) fn upper_covers(page: UpperBound, existing: UpperBound) -> bool {
    match (page, existing) {
        (UpperBound::Unbounded, _) => true,
        (_, UpperBound::Unbounded) => false,
        (UpperBound::Excluded(p), UpperBound::Included(e)) => p > e,
        (UpperBound::Excluded(p), UpperBound::Excluded(e))
        | (UpperBound::Included(p), UpperBound::Included(e) | UpperBound::Excluded(e)) => p >= e,
    }
}

pub(super) fn covered(
    key: Key,
    cursor: Option<Key>,
    far: Option<Key>,
    order: HistoryOrder,
    has_more: bool,
) -> bool {
    match order {
        HistoryOrder::Desc => {
            cursor.is_none_or(|cursor| key < cursor)
                && (!has_more || far.is_some_and(|far| key >= far))
        }
        HistoryOrder::Asc => {
            cursor.is_none_or(|cursor| key > cursor)
                && (!has_more || far.is_some_and(|far| key <= far))
        }
    }
}

pub(super) fn snapshot_chunks_and_gaps(
    model_chunks: &[Chunk],
    empty_before: Option<&HistoryEmptyAnchor>,
    empty_after: Option<&HistoryEmptyAnchor>,
    generation: u64,
) -> (Vec<HistoryChunk>, Vec<HistoryGap>) {
    let mut chunks = Vec::with_capacity(model_chunks.len());
    let mut gaps = Vec::new();
    for (index, chunk) in model_chunks.iter().enumerate() {
        chunks.push(HistoryChunk {
            first: chunk.first.cursor(),
            last: chunk.last.cursor(),
            oldest_anchor: chunk.oldest_anchor.as_ref().map(Point::cursor),
            newest_anchor: chunk.newest_anchor.as_ref().map(Point::cursor),
            oldest_complete: chunk.oldest_complete,
            newest_complete: chunk.newest_complete,
            generation: (chunk.generation != 0).then_some(Generation(chunk.generation)),
            live_generation: (chunk.live_generation != 0)
                .then_some(Generation(chunk.live_generation)),
        });
        let left_empty = index == 0
            && empty_before.is_some_and(|before| {
                before.generation.is_some()
                    && chunk.first.key().is_ok_and(|first| first <= before.key)
            });
        let left_bridged =
            left_empty || (index > 0 && seam_proven(&model_chunks[index - 1], chunk));
        let right_empty = index + 1 == model_chunks.len()
            && empty_after.is_some_and(|after| {
                after.generation == Some(Generation(generation))
                    && chunk.last.key().is_ok_and(|last| last >= after.key)
            });
        let right_bridged = right_empty
            || (index + 1 < model_chunks.len() && seam_proven(chunk, &model_chunks[index + 1]));
        for (complete, bridged, point, order) in [
            (
                chunk.oldest_complete,
                left_bridged,
                &chunk.first,
                HistoryOrder::Desc,
            ),
            (
                chunk.newest_complete,
                right_bridged,
                &chunk.last,
                HistoryOrder::Asc,
            ),
        ] {
            if !complete && !bridged {
                gaps.push(HistoryGap {
                    cursor: point.cursor(),
                    order,
                });
            }
        }
    }
    (chunks, gaps)
}

pub(super) fn snapshot_empty_anchor(
    anchor: Option<&EmptyAnchor>,
) -> Result<Option<HistoryEmptyAnchor>, HistoryError> {
    anchor
        .map(|anchor| {
            Ok(HistoryEmptyAnchor {
                key: anchor.point.key()?,
                generation: (anchor.generation != 0).then_some(Generation(anchor.generation)),
            })
        })
        .transpose()
}
