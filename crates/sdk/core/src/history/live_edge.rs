//! Position-aware live membership and live-edge evidence; field mutations and server pages live elsewhere.

use papo_api::{HistoryOrder, wire::Message};

use super::{
    HistoryError,
    merge::Ticket,
    model::{Chunk, Model},
    ordering::Point,
};

impl Model {
    pub(super) fn live_message(
        &mut self,
        message: &Message,
        generation: u64,
        revision: u64,
    ) -> Result<(), HistoryError> {
        let point = Point::from(&message.cursor().ok_or(HistoryError::InvalidPage)?);
        let key = point.key()?;
        let id = key.id_bytes();
        let channel = Self::id(
            message
                .channel_id
                .as_deref()
                .ok_or(HistoryError::InvalidPage)?,
        )?;
        if let Some(entry) = self.entries.get(&id) {
            if entry.deleted || entry.timeline {
                return Ok(());
            }
            if entry
                .point
                .as_ref()
                .is_some_and(|old| old.key().ok() != Some(key))
            {
                return Err(HistoryError::InvalidPage);
            }
        }
        if let Some(entry) = self
            .entries
            .get_mut(&id)
            .filter(|entry| entry.message.is_some())
        {
            // A fetched pin can precede its journaled creation event; promotion keeps its richer payload.
            entry.timeline = true;
            entry.revision = revision;
            self.order.insert(key, id);
            self.dirty.insert(id);
        } else {
            let ticket = Ticket {
                channel,
                generation,
                revision: 0,
                cursor: None,
                order: HistoryOrder::Desc,
                limit: 1,
            };
            self.merge_message(message, &point, &ticket, revision)?;
        }
        // An insertion inside an existing run changes no interval boundary. In particular,
        // UUID order at an equal timestamp never proves an unknown earlier interval.
        if self
            .chunks
            .iter()
            .any(|chunk| chunk.contains(key).unwrap_or(false))
        {
            return Ok(());
        }
        if let Some(newest) = self.chunks.last_mut()
            && generation != 0
            && newest.live_generation == generation
            && key > newest.last.key()?
        {
            newest.last = point.clone();
            self.empty_generation = 0;
            if self
                .empty_after
                .as_ref()
                .is_some_and(|after| after.point.key().is_ok_and(|ak| key > ak))
            {
                self.empty_after = None;
            }
            return self.record_live_arrival(generation, &point);
        }
        self.insert_live_chunk(id, key, point, generation)
    }

    fn insert_live_chunk(
        &mut self,
        id: [u8; 16],
        key: crate::ChronologicalKey,
        point: Point,
        generation: u64,
    ) -> Result<(), HistoryError> {
        let newest = self
            .chunks
            .last()
            .is_none_or(|chunk| chunk.last.key().is_ok_and(|last| key > last));
        let oldest = self
            .chunks
            .first()
            .is_none_or(|chunk| chunk.first.key().is_ok_and(|first| key < first));
        let empty_proven =
            generation != 0 && self.chunks.is_empty() && self.empty_generation == generation;
        // An unvalidated local recovery (`generation == 0`) must never consume or upgrade coverage.
        let empty_before_proven = generation != 0
            && oldest
            && self.empty_before.as_ref().is_some_and(|before| {
                before.generation != 0 && before.point.key().is_ok_and(|bk| key == bk)
            });
        if empty_before_proven {
            self.empty_before = None;
        }
        let oldest_complete = empty_proven || empty_before_proven;
        let anchored_after = if newest && !oldest_complete && generation != 0 {
            self.empty_after
                .take()
                .filter(|after| {
                    after.generation == generation && after.point.key().is_ok_and(|ak| ak < key)
                })
                .map(|after| after.point)
        } else {
            None
        };
        if self
            .empty_after
            .as_ref()
            .is_some_and(|after| after.point.key().is_ok_and(|ak| key > ak))
        {
            self.empty_after = None;
        }
        if self
            .empty_before
            .as_ref()
            .is_some_and(|before| before.point.key().is_ok_and(|bk| key < bk))
        {
            self.empty_before = None;
        }
        for chunk in &mut self.chunks {
            if chunk
                .oldest_anchor
                .as_ref()
                .is_some_and(|anchor| anchor.key().is_ok_and(|ak| ak <= key))
                && key < chunk.first.key()?
            {
                chunk.oldest_anchor = None;
            }
            if chunk
                .newest_anchor
                .as_ref()
                .is_some_and(|anchor| anchor.key().is_ok_and(|bk| key <= bk))
                && key > chunk.last.key()?
            {
                chunk.newest_anchor = None;
            }
        }
        if newest && let Some(chunk) = self.chunks.last_mut() {
            chunk.newest_complete = false;
            chunk.newest_anchor = None;
            chunk.live_generation = 0;
        }
        if oldest && let Some(chunk) = self.chunks.first_mut() {
            chunk.oldest_complete = false;
            chunk.oldest_anchor = None;
        }
        if newest && generation != 0 {
            self.record_live_arrival(generation, &point)?;
        }
        self.empty_generation = 0;
        self.chunks.push(Chunk {
            id,
            first: point.clone(),
            last: point,
            oldest_anchor: anchored_after,
            newest_anchor: None,
            oldest_complete,
            newest_complete: newest && generation != 0,
            generation,
            live_generation: if newest { generation } else { 0 },
        });
        self.chunks = super::chunk_merge::coalesce_chunks(std::mem::take(&mut self.chunks))?;
        Ok(())
    }

    fn record_live_arrival(&mut self, generation: u64, point: &Point) -> Result<(), HistoryError> {
        if self.live_arrival_generation != generation {
            self.live_arrival_generation = generation;
            self.live_arrival_floor = Some(point.clone());
            return Ok(());
        }
        if let Some(floor) = &self.live_arrival_floor {
            if point.key()? < floor.key()? {
                self.live_arrival_floor = Some(point.clone());
            }
        } else {
            self.live_arrival_floor = Some(point.clone());
        }
        Ok(())
    }
}
