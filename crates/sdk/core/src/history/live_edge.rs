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
        let channel = Self::id(
            message
                .channel_id
                .as_deref()
                .ok_or(HistoryError::InvalidPage)?,
        )?;
        if let Some(entry) = self.entries.get(&key.id) {
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
            .get_mut(&key.id)
            .filter(|entry| entry.message.is_some())
        {
            // A fetched pin can precede its journaled creation event; promotion keeps its richer payload.
            entry.timeline = true;
            entry.revision = revision;
            self.order.insert(key, key.id);
            self.dirty.insert(key.id);
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
            newest.last = point;
            return Ok(());
        }
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
        if newest && let Some(chunk) = self.chunks.last_mut() {
            chunk.newest_complete = false;
            chunk.live_generation = 0;
        }
        if oldest && let Some(chunk) = self.chunks.first_mut() {
            chunk.oldest_complete = false;
        }
        self.chunks.push(Chunk {
            id: key.id,
            first: point.clone(),
            last: point,
            oldest_complete: empty_proven,
            newest_complete: newest && generation != 0,
            generation,
            live_generation: if newest { generation } else { 0 },
        });
        self.chunks.sort_by_key(|chunk| chunk.first.key().ok());
        Ok(())
    }
}
