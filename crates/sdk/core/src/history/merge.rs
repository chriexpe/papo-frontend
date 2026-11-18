//! Checked page validation and message mutation merging; chunk interval coverage lives in `coverage`.

use std::collections::BTreeSet;

use papo_api::{HistoryOrder, wire::Message};

use super::{
    HistoryError,
    chunk_merge::covered,
    model::{Entry, Model},
    mutations::edit_timestamp_us,
    ordering::Point,
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
                edit_timestamp_us(edited_at)?;
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
                .get(&key.id_bytes())
                .and_then(|entry| entry.point.as_ref())
                .is_some_and(|existing| existing.key().ok() != Some(key))
            {
                return Err(HistoryError::InvalidPage);
            }
        }
        let revision = self.next_revision()?;
        let present: BTreeSet<_> = points
            .iter()
            .map(Point::id_bytes)
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
        let id = key.id_bytes();
        let entry = self
            .entries
            .entry(id)
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
                .map(edit_timestamp_us)
                .transpose()?;
            let new_edit = incoming
                .edited_at
                .as_deref()
                .map(edit_timestamp_us)
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
            let pending = edit_timestamp_us(edited_at)?;
            let fetched = incoming
                .edited_at
                .as_deref()
                .map(edit_timestamp_us)
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
        self.order.insert(key, id);
        self.dirty.insert(id);
        Ok(())
    }
}
