//! Non-empty page interval merging, tail extension, and unvalidated-cache splitting.
//! Empty-page anchors live in `empty_coverage`; bound algebra and chunk coalescing live in `chunk_merge`.

use std::{
    collections::BTreeSet,
    ops::Bound::{Excluded, Included, Unbounded},
};

use papo_api::HistoryOrder;
use sha2::{Digest, Sha256};

use super::{
    HistoryError,
    chunk_merge::{
        LowerBound, UpperBound, absorb_chunk, coalesce_chunks, covered, lower_covers, upper_covers,
    },
    merge::Ticket,
    model::{Chunk, Model},
    ordering::{Key, Point},
};

struct PageRefreshBounds {
    anchor: Option<Key>,
    far: Option<Key>,
    page_lower: LowerBound,
    page_upper: UpperBound,
    order: HistoryOrder,
    has_more: bool,
    generation: u64,
}

impl PageRefreshBounds {
    fn refreshes(&self, chunk: &Chunk) -> Result<bool, HistoryError> {
        Ok(covered(
            chunk.first.key()?,
            self.anchor,
            self.far,
            self.order,
            self.has_more,
        ) && covered(
            chunk.last.key()?,
            self.anchor,
            self.far,
            self.order,
            self.has_more,
        ) && (chunk.generation != 0
            || (lower_covers(self.page_lower, chunk.lower_bound()?)
                && upper_covers(
                    self.page_upper,
                    chunk.upper_bound_in(Some(self.generation))?,
                ))))
    }
}

impl Model {
    pub(super) fn merge_coverage(
        &mut self,
        ticket: &Ticket,
        points: &[Point],
        has_more: bool,
    ) -> Result<(), HistoryError> {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return Err(HistoryError::InvalidPage);
        };
        let (first, last) = match ticket.order {
            HistoryOrder::Desc => (last.clone(), first.clone()),
            HistoryOrder::Asc => (first.clone(), last.clone()),
        };
        let first_key = first.key()?;
        let last_key = last.key()?;
        let (empty_before_meets_first, empty_after_meets_last) =
            self.consume_empty_anchors_for_merge(ticket, first_key, last_key, has_more)?;
        // Merge the bounded interval (messages plus anchor) before extending `!has_more` so
        // concurrent out-of-order responses cannot bridge an unmerged chunk across missing rows.
        let oldest_complete = (ticket.order == HistoryOrder::Asc && ticket.cursor.is_none())
            || empty_before_meets_first;
        let newest_complete = (ticket.order == HistoryOrder::Desc && ticket.cursor.is_none())
            || empty_after_meets_last;
        let mut joined = Chunk {
            id: first_key.id_bytes(),
            first,
            last,
            oldest_anchor: (!oldest_complete && ticket.order == HistoryOrder::Asc)
                .then(|| ticket.cursor.clone())
                .flatten(),
            newest_anchor: (!newest_complete && ticket.order == HistoryOrder::Desc)
                .then(|| ticket.cursor.clone())
                .flatten(),
            oldest_complete,
            newest_complete,
            generation: ticket.generation,
            live_generation: if newest_complete {
                ticket.generation
            } else {
                0
            },
        };
        self.prune_omitted_tombstone_chunks(&joined)?;
        let mut remaining =
            self.absorb_overlapping_chunks(ticket, points, has_more, &mut joined)?;
        self.extend_complete_tails(ticket, has_more, &mut joined, &mut remaining)?;
        self.empty_generation = 0;
        remaining.push(joined);
        self.chunks = coalesce_chunks(remaining)?;
        Ok(())
    }

    fn prune_omitted_tombstone_chunks(&mut self, joined: &Chunk) -> Result<(), HistoryError> {
        let j_first = joined.first.key()?;
        let j_last = joined.last.key()?;
        if joined.oldest_complete && self.order.range(..j_first).next().is_none() {
            self.chunks
                .retain(|chunk| chunk.last.key().ok() >= Some(j_first));
        }
        if joined.newest_complete
            && self
                .order
                .range((Excluded(j_last), Unbounded))
                .next()
                .is_none()
        {
            self.chunks
                .retain(|chunk| chunk.first.key().ok() <= Some(j_last));
        }
        Ok(())
    }

    fn absorb_overlapping_chunks(
        &mut self,
        ticket: &Ticket,
        points: &[Point],
        has_more: bool,
        joined: &mut Chunk,
    ) -> Result<Vec<Chunk>, HistoryError> {
        let page_lower = if (ticket.order == HistoryOrder::Asc && ticket.cursor.is_none())
            || (ticket.order == HistoryOrder::Desc && !has_more)
        {
            LowerBound::Unbounded
        } else {
            joined.lower_bound()?
        };
        // A REST page's live tail (`newest_complete`) expires across reconnect, so it cannot
        // promote an unvalidated cached interval extending strictly beyond `joined.upper_bound_in(Some(0))`.
        let page_upper = joined.upper_bound_in(Some(0))?;
        let bounds = PageRefreshBounds {
            anchor: ticket.cursor.as_ref().map(Point::key).transpose()?,
            far: points.last().map(Point::key).transpose()?,
            page_lower,
            page_upper,
            order: ticket.order,
            has_more,
            generation: ticket.generation,
        };
        let mut remaining = std::mem::take(&mut self.chunks);
        loop {
            let mut merged_any = false;
            let mut next_remaining = Vec::with_capacity(remaining.len());
            for mut chunk in remaining {
                let refreshed = bounds.refreshes(&chunk)?;
                if joined.touches_or_overlaps_in(&chunk, Some(ticket.generation))?
                    && (refreshed || chunk.generation != 0)
                {
                    if chunk.live_generation != ticket.generation
                        && !joined.newest_complete
                        && covered(
                            chunk.last.key()?,
                            bounds.anchor,
                            bounds.far,
                            ticket.order,
                            has_more,
                        )
                    {
                        chunk.newest_complete = false;
                        chunk.live_generation = 0;
                    }
                    absorb_chunk(joined, chunk, refreshed)?;
                    merged_any = true;
                } else {
                    next_remaining.push(chunk);
                }
            }
            remaining = next_remaining;
            if !merged_any {
                break;
            }
        }
        // Any remaining unvalidated chunk overlapping or touching `joined` keeps only its
        // unrefreshed live tails so partial revalidation preserves `joined.generation`.
        let mut split_remaining = Vec::with_capacity(remaining.len());
        let mut used_ids: BTreeSet<[u8; 16]> =
            remaining.iter().map(|c| c.id).chain([joined.id]).collect();
        for chunk in remaining {
            if chunk.generation == 0
                && joined.touches_or_overlaps_in(&chunk, Some(ticket.generation))?
            {
                if bounds.refreshes(&chunk)? {
                    absorb_chunk(joined, chunk, true)?;
                } else if chunk.overlaps_entries(joined)?
                    || covered(
                        chunk.first.key()?,
                        bounds.anchor,
                        bounds.far,
                        ticket.order,
                        has_more,
                    )
                    || covered(
                        chunk.last.key()?,
                        bounds.anchor,
                        bounds.far,
                        ticket.order,
                        has_more,
                    )
                {
                    self.split_unvalidated_chunk(
                        chunk,
                        joined,
                        &mut used_ids,
                        &mut split_remaining,
                    )?;
                } else {
                    split_remaining.push(chunk);
                }
            } else {
                split_remaining.push(chunk);
            }
        }
        Ok(split_remaining)
    }

    fn split_unvalidated_chunk(
        &self,
        chunk: Chunk,
        joined: &Chunk,
        used_ids: &mut BTreeSet<[u8; 16]>,
        out: &mut Vec<Chunk>,
    ) -> Result<(), HistoryError> {
        let c_first = chunk.first.key()?;
        let c_last = chunk.last.key()?;
        let j_first = joined.first.key()?;
        let j_last = joined.last.key()?;
        if c_first < j_first {
            let mut left_entries = self.order.range((Included(c_first), Excluded(j_first)));
            if let Some((_, &first_id)) = left_entries.next() {
                let last_id = left_entries.next_back().map_or(first_id, |(_, &id)| id);
                let last = self
                    .entries
                    .get(&last_id)
                    .and_then(|entry| entry.point.clone())
                    .ok_or(HistoryError::InvalidPage)?;
                let id = unique_chunk_id(first_id, used_ids);
                used_ids.insert(id);
                out.push(Chunk {
                    id,
                    first: chunk.first.clone(),
                    last,
                    oldest_anchor: chunk.oldest_anchor.clone(),
                    newest_anchor: None,
                    oldest_complete: chunk.oldest_complete,
                    newest_complete: false,
                    generation: 0,
                    live_generation: 0,
                });
            }
        }
        if j_last < c_last {
            let mut right_entries = self.order.range((Excluded(j_last), Included(c_last)));
            if let Some((_, &first_id)) = right_entries.next() {
                let first = self
                    .entries
                    .get(&first_id)
                    .and_then(|entry| entry.point.clone())
                    .ok_or(HistoryError::InvalidPage)?;
                let id = unique_chunk_id(first_id, used_ids);
                used_ids.insert(id);
                out.push(Chunk {
                    id,
                    first,
                    last: chunk.last,
                    oldest_anchor: None,
                    newest_anchor: chunk.newest_anchor,
                    oldest_complete: false,
                    newest_complete: chunk.newest_complete,
                    generation: 0,
                    live_generation: 0,
                });
            }
        }
        Ok(())
    }

    fn extend_complete_tails(
        &mut self,
        ticket: &Ticket,
        has_more: bool,
        joined: &mut Chunk,
        remaining: &mut Vec<Chunk>,
    ) -> Result<(), HistoryError> {
        if !has_more && ticket.order == HistoryOrder::Desc {
            let j_first = joined.first.key()?;
            remaining.retain(|chunk| {
                !(chunk.last.key().ok() < Some(j_first)
                    && self.order.range(..j_first).next().is_none())
            });
            if !remaining
                .iter()
                .any(|chunk| chunk.first.key().ok() < Some(j_first))
            {
                joined.oldest_complete = true;
                joined.oldest_anchor = None;
            }
        }
        if !has_more && ticket.order == HistoryOrder::Asc {
            let j_last = joined.last.key()?;
            remaining.retain(|chunk| {
                !(chunk.first.key().ok() > Some(j_last)
                    && self
                        .order
                        .range((Excluded(j_last), Unbounded))
                        .next()
                        .is_none())
            });
            if !remaining
                .iter()
                .any(|chunk| chunk.last.key().ok() > Some(j_last))
            {
                joined.newest_complete = true;
                joined.newest_anchor = None;
                joined.live_generation = ticket.generation;
                self.server_live_generation = ticket.generation;
            }
        }
        if joined.newest_complete {
            self.server_live_generation = self.server_live_generation.max(joined.live_generation);
            self.empty_after = None;
        }
        if joined.oldest_complete {
            self.empty_before = None;
        }
        Ok(())
    }
}

fn unique_chunk_id(preferred: [u8; 16], used: &BTreeSet<[u8; 16]>) -> [u8; 16] {
    if !used.contains(&preferred) {
        return preferred;
    }
    for salt in 0u64.. {
        let mut hasher = Sha256::new();
        hasher.update(b"papo-chunk-split/v1");
        hasher.update(preferred);
        hasher.update(salt.to_be_bytes());
        let digest = hasher.finalize();
        let mut candidate = [0u8; 16];
        candidate.copy_from_slice(&digest[..16]);
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}
