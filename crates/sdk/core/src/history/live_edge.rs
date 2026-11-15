//! Position-aware live membership and live-edge evidence; field mutations and server pages live elsewhere.

use papo_api::{HistoryOrder, wire::Message};

use super::{
    HistoryError,
    merge::Ticket,
    message_key::MessageKey,
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
        if let Some(entry) = self.entries.get(&key.id_bytes()) {
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
            .get_mut(&key.id_bytes())
            .filter(|entry| entry.message.is_some())
        {
            // A fetched pin can precede its journaled creation event; promotion keeps its richer payload.
            entry.timeline = true;
            entry.revision = revision;
            self.order.insert(key, key.id_bytes());
            self.dirty.insert(key.id_bytes());
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
        if self.hold_anchor(&point, key) {
            return Ok(());
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
            if newest.is_anchor_only() {
                newest.first = point.clone();
            }
            // The run now reaches the arrival; an anchor it has passed proves nothing more.
            if let Some(anchor) = &newest.newest_anchor
                && anchor.key()? <= key
            {
                newest.newest_anchor = None;
            }
            newest.last = point.clone();
            return self.record_live_arrival(generation, &point);
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
        if newest && generation != 0 {
            self.record_live_arrival(generation, &point)?;
        }
        self.chunks.push(Chunk {
            id: key.id_bytes(),
            first: point.clone(),
            last: point,
            oldest_complete: empty_proven,
            newest_complete: newest && generation != 0,
            generation,
            live_generation: if newest { generation } else { 0 },
            oldest_anchor: None,
            newest_anchor: None,
        });
        self.chunks.sort_by_key(|chunk| chunk.first.key().ok());
        Ok(())
    }

    /// A message at an anchor-only run's cursor makes that cursor a held boundary. The run
    /// keeps only what stays justified: an empty `Asc` proof `(X, ∞)` becomes `[X, ∞)` in its
    /// live generation, an empty `Desc` proof `(-∞, X)` becomes `(-∞, X]`. Returns whether a
    /// run was converted.
    fn hold_anchor(&mut self, point: &Point, key: MessageKey) -> bool {
        let Some(chunk) = self.chunks.iter_mut().find(|chunk| {
            chunk.first == chunk.last
                && chunk.first.key().is_ok_and(|first| first == key)
                && chunk.is_anchor_only()
        }) else {
            return false;
        };
        chunk.first = point.clone();
        chunk.last = point.clone();
        chunk.oldest_anchor = None;
        chunk.newest_anchor = None;
        true
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
