//! Checked page reconciliation and contiguous-run joining; events and persistence live elsewhere.

use std::collections::BTreeSet;

use papo_api::{HistoryOrder, wire::Message};

use super::{
    HistoryError,
    model::{Chunk, Entry, Model},
    ordering::{Key, Point, timestamp_us},
};

pub(super) struct Ticket {
    pub channel: [u8; 16],
    pub generation: u64,
    pub revision: u64,
    pub cursor: Option<Point>,
    pub order: HistoryOrder,
    pub limit: usize,
}

impl Ticket {
    pub fn validate(
        &self,
        messages: &[Message],
        has_more: bool,
    ) -> Result<Vec<Point>, HistoryError> {
        if self.limit == 0
            || self.limit > 100
            || messages.len() > self.limit
            || (messages.is_empty() && has_more)
        {
            return Err(HistoryError::InvalidPage);
        }
        let mut previous = self.cursor.as_ref().map(Point::key).transpose()?;
        let mut points = Vec::with_capacity(messages.len());
        for message in messages {
            if Model::id(
                message
                    .channel_id
                    .as_deref()
                    .ok_or(HistoryError::InvalidPage)?,
            )? != self.channel
            {
                return Err(HistoryError::InvalidPage);
            }
            let point = Point::from(&message.cursor().ok_or(HistoryError::InvalidPage)?);
            let key = point.key()?;
            if previous.is_some_and(|previous| match self.order {
                HistoryOrder::Desc => key >= previous,
                HistoryOrder::Asc => key <= previous,
            }) {
                return Err(HistoryError::InvalidPage);
            }
            if let Some(edited_at) = &message.edited_at {
                timestamp_us(edited_at)?;
            }
            previous = Some(key);
            points.push(point);
        }
        Ok(points)
    }
}

impl Model {
    /// Validate the entire page before mutation; a server omission deletes only its covered range.
    pub fn merge_page(
        &mut self,
        ticket: &Ticket,
        messages: &[Message],
        has_more: bool,
    ) -> Result<(), HistoryError> {
        let points = ticket.validate(messages, has_more)?;
        for point in &points {
            let key = point.key()?;
            if self
                .entries
                .get(&key.id)
                .and_then(|entry| entry.point.as_ref())
                .is_some_and(|existing| existing.key().ok() != Some(key))
            {
                return Err(HistoryError::InvalidPage);
            }
        }
        let revision = self.next_revision()?;
        let present: BTreeSet<_> = points
            .iter()
            .map(|point| point.key().map(|key| key.id))
            .collect::<Result<_, _>>()?;
        let cursor = ticket.cursor.as_ref().map(Point::key).transpose()?;
        let far = points.last().map(Point::key).transpose()?;
        let removed: Vec<_> = self
            .order
            .iter()
            .filter(|(key, id)| {
                !present.contains(*id)
                    && self
                        .entries
                        .get(*id)
                        .is_some_and(|entry| entry.revision <= ticket.revision)
                    && covered(**key, cursor, far, ticket.order, has_more)
            })
            .map(|(key, id)| (*key, *id))
            .collect();
        self.record_deletions(revision, removed.iter().map(|(_, id)| *id).collect());
        for (key, id) in removed {
            self.order.remove(&key);
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.message = None;
                entry.deleted = true;
                entry.timeline = false;
                entry.revision = revision;
            }
            self.dirty.insert(id);
        }
        for (message, point) in messages.iter().zip(&points) {
            self.merge_message(message, point, ticket, revision)?;
        }
        if points.is_empty() {
            self.finish_empty(ticket, has_more)?;
            return Ok(());
        }
        self.merge_coverage(ticket, &points, has_more)
    }

    pub fn merge_message(
        &mut self,
        message: &Message,
        point: &Point,
        ticket: &Ticket,
        revision: u64,
    ) -> Result<(), HistoryError> {
        let key = point.key()?;
        let entry = self
            .entries
            .entry(key.id)
            .or_insert_with(|| Entry::empty(revision));
        if entry.deleted {
            return Ok(());
        }
        if entry
            .point
            .as_ref()
            .is_some_and(|existing| existing.key().ok() != Some(key))
        {
            return Err(HistoryError::InvalidPage);
        }
        let mut incoming = message.clone();
        if let Some(existing) = &entry.message {
            let old_edit = existing
                .edited_at
                .as_deref()
                .map(timestamp_us)
                .transpose()?;
            let new_edit = incoming
                .edited_at
                .as_deref()
                .map(timestamp_us)
                .transpose()?;
            if old_edit.is_some() && old_edit >= new_edit {
                incoming.content.clone_from(&existing.content);
                incoming.edited_at.clone_from(&existing.edited_at);
            }
            if entry.reaction_revision > ticket.revision {
                incoming.reactions.clone_from(&existing.reactions);
            }
            if entry.embeds_revision > ticket.revision {
                incoming.embeds.clone_from(&existing.embeds);
            }
        }
        if let Some((edited_at, content)) = &entry.edit {
            let pending = timestamp_us(edited_at)?;
            let fetched = incoming
                .edited_at
                .as_deref()
                .map(timestamp_us)
                .transpose()?;
            if fetched.is_none_or(|fetched| pending > fetched) {
                incoming.edited_at = Some(edited_at.clone());
                incoming.content = Some(content.clone());
            }
        }
        if entry.embeds_revision > ticket.revision
            && let Some(embeds) = &entry.pending_embeds
        {
            incoming.embeds = Some(embeds.clone());
        }
        if entry.reaction_revision > ticket.revision {
            for total in &entry.pending_reactions {
                super::mutations::replace_total(
                    incoming.reactions.get_or_insert_with(Vec::new),
                    total,
                );
            }
        }
        entry.message = Some(incoming);
        entry.point = Some(point.clone());
        entry.timeline = true;
        entry.revision = revision;
        entry.validated_generation = ticket.generation;
        self.order.insert(key, key.id);
        self.dirty.insert(key.id);
        Ok(())
    }

    fn finish_empty(&mut self, ticket: &Ticket, has_more: bool) -> Result<(), HistoryError> {
        if has_more {
            return Err(HistoryError::InvalidPage);
        }
        let Some(cursor) = &ticket.cursor else {
            self.chunks.clear();
            self.empty_generation = ticket.generation;
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
                self.chunks.push(Chunk {
                    id: *first_id,
                    first,
                    last,
                    oldest_complete: true,
                    newest_complete: true,
                    generation: ticket.generation,
                    live_generation: ticket.generation,
                });
            }
            return Ok(());
        };
        let key = cursor.key()?;
        for chunk in &mut self.chunks {
            if chunk.contains(key)? {
                match ticket.order {
                    HistoryOrder::Desc => chunk.oldest_complete = true,
                    HistoryOrder::Asc => {
                        chunk.newest_complete = true;
                        chunk.live_generation = ticket.generation;
                    }
                }
            }
        }
        Ok(())
    }

    fn merge_coverage(
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
        let mut joined = Chunk {
            id: first.key()?.id,
            first,
            last,
            oldest_complete: ticket.order == HistoryOrder::Desc && !has_more,
            newest_complete: (ticket.order == HistoryOrder::Desc && ticket.cursor.is_none())
                || (ticket.order == HistoryOrder::Asc && !has_more),
            generation: ticket.generation,
            live_generation: 0,
        };
        if joined.newest_complete {
            joined.live_generation = ticket.generation;
        }
        let anchor = ticket.cursor.as_ref().map(Point::key).transpose()?;
        let mut retained = Vec::new();
        for chunk in self.chunks.drain(..) {
            let overlap = chunk.first.key()? <= joined.last.key()?
                && joined.first.key()? <= chunk.last.key()?;
            let attached = anchor.is_some_and(|anchor| chunk.contains(anchor).unwrap_or(false));
            if !overlap && !attached {
                retained.push(chunk);
                continue;
            }
            let refreshed = covered(
                chunk.first.key()?,
                anchor,
                points.last().map(Point::key).transpose()?,
                ticket.order,
                has_more,
            ) && covered(
                chunk.last.key()?,
                anchor,
                points.last().map(Point::key).transpose()?,
                ticket.order,
                has_more,
            );
            joined.id = joined.id.min(chunk.id);
            if chunk.first.key()? < joined.first.key()? {
                joined.first = chunk.first;
            }
            if chunk.last.key()? > joined.last.key()? {
                joined.last = chunk.last;
            }
            joined.oldest_complete |= chunk.oldest_complete;
            joined.newest_complete |= chunk.newest_complete;
            joined.live_generation = joined.live_generation.max(chunk.live_generation);
            if !refreshed {
                joined.generation = joined.generation.min(chunk.generation);
            }
        }
        retained.push(joined);
        retained.sort_by_key(|chunk| chunk.first.key().ok());
        self.chunks = retained;
        Ok(())
    }
}

fn covered(
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
