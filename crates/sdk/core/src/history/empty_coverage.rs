//! Empty-page anchor tracking and boundary closure for channel history coverage.
//! Non-empty chunk merging lives in `coverage`; bound algebra lives in `chunk_merge`.

use std::ops::Bound::{Excluded, Included, Unbounded};

use papo_api::HistoryOrder;

use super::{
    HistoryError,
    merge::Ticket,
    model::{Chunk, EmptyAnchor, Model},
    ordering::{Key, Point},
};

impl Model {
    pub(super) fn finish_empty(
        &mut self,
        ticket: &Ticket,
        has_more: bool,
    ) -> Result<(), HistoryError> {
        if has_more {
            return Err(HistoryError::InvalidPage);
        }
        let Some(cursor) = &ticket.cursor else {
            self.chunks.clear();
            self.empty_before = None;
            self.empty_after = None;
            self.server_live_generation = ticket.generation;
            if let (Some((_, first_id)), Some((_, last_id))) =
                (self.order.first_key_value(), self.order.last_key_value())
            {
                let first = self.entries[first_id]
                    .point
                    .clone()
                    .ok_or(HistoryError::InvalidPage)?;
                let last = self.entries[last_id]
                    .point
                    .clone()
                    .ok_or(HistoryError::InvalidPage)?;
                self.empty_generation = 0;
                self.chunks.push(Chunk {
                    id: *first_id,
                    first,
                    last,
                    oldest_anchor: None,
                    newest_anchor: None,
                    oldest_complete: true,
                    newest_complete: true,
                    generation: ticket.generation,
                    live_generation: ticket.generation,
                });
            } else {
                self.empty_generation = ticket.generation;
            }
            return Ok(());
        };
        let key = cursor.key()?;
        let has_live_outside = match ticket.order {
            HistoryOrder::Desc => self.order.range(..key).next().is_some(),
            HistoryOrder::Asc => self
                .order
                .range((Excluded(key), Unbounded))
                .next()
                .is_some(),
        };
        if has_live_outside {
            return Ok(());
        }
        match ticket.order {
            HistoryOrder::Desc => {
                // Chunks wholly < key hold only tombstones omitted by this empty response.
                self.chunks
                    .retain(|chunk| chunk.last.key().ok() >= Some(key));
            }
            HistoryOrder::Asc => {
                self.chunks
                    .retain(|chunk| chunk.first.key().ok() <= Some(key));
            }
        }
        if let Some(idx) = self
            .chunks
            .iter()
            .position(|chunk| chunk.contains(key).unwrap_or(false))
        {
            if self.chunks[idx].generation != 0 {
                match ticket.order {
                    HistoryOrder::Desc if idx == 0 => {
                        self.chunks[0].oldest_complete = true;
                        self.chunks[0].oldest_anchor = None;
                        self.empty_before = None;
                    }
                    HistoryOrder::Asc if idx + 1 == self.chunks.len() => {
                        self.chunks[idx].newest_complete = true;
                        self.chunks[idx].newest_anchor = None;
                        self.chunks[idx].live_generation = ticket.generation;
                        self.server_live_generation = ticket.generation;
                        self.empty_after = None;
                    }
                    _ => {}
                }
                return Ok(());
            }
            match ticket.order {
                HistoryOrder::Desc if idx == 0 => {
                    self.trim_unvalidated_head_on_empty_desc(cursor, key, ticket.generation)?;
                }
                HistoryOrder::Asc if idx + 1 == self.chunks.len() => {
                    self.trim_unvalidated_tail_on_empty_asc(cursor, key, ticket.generation)?;
                }
                _ => {}
            }
            return Ok(());
        }
        match ticket.order {
            HistoryOrder::Desc => self.record_empty_before(cursor, key, ticket.generation),
            HistoryOrder::Asc => self.record_empty_after(cursor, key, ticket.generation),
        }
        Ok(())
    }

    fn trim_unvalidated_head_on_empty_desc(
        &mut self,
        cursor: &Point,
        key: Key,
        generation: u64,
    ) -> Result<(), HistoryError> {
        let c_last = self.chunks[0].last.key()?;
        if let Some((_, &live_id)) = self.order.range((Included(key), Included(c_last))).next() {
            let live_point = self
                .entries
                .get(&live_id)
                .and_then(|entry| entry.point.clone())
                .ok_or(HistoryError::InvalidPage)?;
            self.chunks[0].first = live_point;
            self.chunks[0].oldest_anchor = None;
            self.chunks[0].oldest_complete = false;
        } else {
            self.chunks.remove(0);
        }
        self.record_empty_before(cursor, key, generation);
        Ok(())
    }

    fn trim_unvalidated_tail_on_empty_asc(
        &mut self,
        cursor: &Point,
        key: Key,
        generation: u64,
    ) -> Result<(), HistoryError> {
        let last_idx = self.chunks.len() - 1;
        let c_first = self.chunks[last_idx].first.key()?;
        if let Some((_, &live_id)) = self
            .order
            .range((Included(c_first), Included(key)))
            .next_back()
        {
            let live_point = self
                .entries
                .get(&live_id)
                .and_then(|entry| entry.point.clone())
                .ok_or(HistoryError::InvalidPage)?;
            self.chunks[last_idx].last = live_point;
            self.chunks[last_idx].newest_anchor = None;
            self.chunks[last_idx].newest_complete = false;
            self.chunks[last_idx].live_generation = 0;
        } else {
            self.chunks.pop();
        }
        self.server_live_generation = generation;
        self.record_empty_after(cursor, key, generation);
        Ok(())
    }

    fn record_empty_before(&mut self, cursor: &Point, key: Key, generation: u64) {
        if self
            .chunks
            .first()
            .is_some_and(|chunk| chunk.first.key().is_ok_and(|first| first < key))
            || self.order.range(..key).next().is_some()
        {
            return;
        }
        if let Some(first) = self.chunks.first_mut()
            && first.generation != 0
            && first
                .oldest_anchor
                .as_ref()
                .is_some_and(|anchor| anchor.key().is_ok_and(|ak| ak < key))
        {
            first.oldest_complete = true;
            first.oldest_anchor = None;
            self.empty_before = None;
            return;
        }
        let keep_existing = self
            .empty_before
            .as_ref()
            .filter(|anchor| anchor.generation == generation)
            .and_then(|anchor| anchor.point.key().ok())
            .is_some_and(|ak| ak >= key);
        if !keep_existing {
            self.empty_before = Some(EmptyAnchor {
                point: cursor.clone(),
                generation,
            });
        }
    }

    fn record_empty_after(&mut self, cursor: &Point, key: Key, generation: u64) {
        if self
            .chunks
            .last()
            .is_some_and(|chunk| chunk.last.key().is_ok_and(|last| last > key))
            || self
                .order
                .range((Excluded(key), Unbounded))
                .next()
                .is_some()
        {
            return;
        }
        if let Some(last) = self.chunks.last_mut()
            && last.generation != 0
            && last
                .newest_anchor
                .as_ref()
                .is_some_and(|anchor| anchor.key().is_ok_and(|bk| bk > key))
        {
            last.newest_complete = true;
            last.newest_anchor = None;
            last.live_generation = generation;
            self.server_live_generation = generation;
            self.empty_after = None;
            return;
        }
        let keep_existing = self
            .empty_after
            .as_ref()
            .filter(|anchor| anchor.generation == generation)
            .and_then(|anchor| anchor.point.key().ok())
            .is_some_and(|bk| bk <= key);
        if !keep_existing {
            self.empty_after = Some(EmptyAnchor {
                point: cursor.clone(),
                generation,
            });
        }
    }

    pub(super) fn consume_empty_anchors_for_merge(
        &mut self,
        ticket: &Ticket,
        first_key: Key,
        last_key: Key,
        has_more: bool,
    ) -> Result<(bool, bool), HistoryError> {
        let mut empty_before_meets_first = false;
        if let Some(before) = &self.empty_before {
            let bk = before.point.key()?;
            if first_key < bk || (has_more && ticket.order == HistoryOrder::Desc && first_key <= bk)
            {
                self.empty_before = None;
            } else if first_key == bk && before.generation != 0 {
                empty_before_meets_first = true;
                self.empty_before = None;
            }
        }
        let mut empty_after_meets_last = false;
        if let Some(after) = &self.empty_after {
            let ak = after.point.key()?;
            if last_key > ak || (has_more && ticket.order == HistoryOrder::Asc && last_key >= ak) {
                self.empty_after = None;
            } else if last_key == ak && after.generation == ticket.generation {
                empty_after_meets_last = true;
                self.empty_after = None;
            }
        }
        Ok((empty_before_meets_first, empty_after_meets_last))
    }
}
