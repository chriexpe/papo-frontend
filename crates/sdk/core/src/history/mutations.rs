//! Live message mutation precedence and detached pins; paging and socket journaling live elsewhere.

use std::collections::BTreeSet;

use papo_api::{
    gateway::KnownEvent,
    wire::{Message, MessageReactionSummary},
};

use super::{
    HistoryError,
    merge::Ticket,
    model::{Entry, Model},
    ordering::{Point, timestamp_us},
};

impl Model {
    pub fn event(&mut self, event: &KnownEvent, generation: u64) -> Result<bool, HistoryError> {
        let revision = self.next_revision()?;
        match event {
            KnownEvent::Message(event) => {
                let message = Message {
                    id: Some(event.id.clone()),
                    channel_id: Some(event.channel_id.clone()),
                    author_id: Some(event.author_id.clone()),
                    content: Some(event.content.clone()),
                    created_at: Some(event.created_at.clone()),
                    reply_to: event.reply_to.clone(),
                    webhook_id: event.webhook_id.clone(),
                    external_author: event.external_author.clone(),
                    attachments: Some(event.attachments.clone()),
                    ..Message::default()
                };
                self.live_message(&message, generation, revision)?;
            }
            KnownEvent::MessageEdit(event) => return self.apply_edit(event, revision),
            KnownEvent::MessageDelete(event) => {
                let id = Self::id(&event.id)?;
                let entry = self
                    .entries
                    .entry(id)
                    .or_insert_with(|| Entry::empty(revision));
                if let Some(point) = &entry.point {
                    self.order.remove(&point.key()?);
                }
                entry.message = None;
                entry.deleted = true;
                entry.timeline = false;
                entry.pinned = false;
                entry.edit = None;
                entry.pending_embeds = None;
                entry.pending_reactions.clear();
                entry.revision = revision;
                self.dirty.insert(id);
                self.record_deletions(revision, vec![id]);
            }
            KnownEvent::MessageEmbedsUpdate(event) => {
                let id = Self::id(&event.message_id)?;
                let entry = self
                    .entries
                    .entry(id)
                    .or_insert_with(|| Entry::empty(revision));
                if entry.deleted {
                    return Ok(false);
                }
                entry.pending_embeds = Some(event.embeds.clone());
                if let Some(message) = &mut entry.message {
                    message.embeds = Some(event.embeds.clone());
                }
                entry.embeds_revision = revision;
                entry.revision = revision;
                self.dirty.insert(id);
            }
            KnownEvent::ReactUpdate(event) => return self.apply_reaction(event, revision),
            KnownEvent::MessagePin(event) => {
                let id = Self::id(&event.message_id)?;
                let entry = self
                    .entries
                    .entry(id)
                    .or_insert_with(|| Entry::empty(revision));
                if entry.deleted {
                    return Ok(false);
                }
                entry.pinned = event.is_pinned;
                entry.pin_revision = revision;
                entry.revision = revision;
                self.dirty.insert(id);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn apply_edit(
        &mut self,
        event: &papo_api::gateway::MessageEditEvent,
        revision: u64,
    ) -> Result<bool, HistoryError> {
        let id = Self::id(&event.id)?;
        let edited_at_us = timestamp_us(&event.edited_at)?;
        let entry = self
            .entries
            .entry(id)
            .or_insert_with(|| Entry::empty(revision));
        if entry.deleted {
            return Ok(false);
        }
        let old = entry
            .edit
            .as_ref()
            .map(|(at, _)| timestamp_us(at))
            .transpose()?;
        let fetched = entry
            .message
            .as_ref()
            .and_then(|message| message.edited_at.as_deref())
            .map(timestamp_us)
            .transpose()?;
        if old
            .into_iter()
            .chain(fetched)
            .any(|old| old >= edited_at_us)
        {
            return Ok(false);
        }
        entry.edit = Some((event.edited_at.clone(), event.content.clone()));
        if let Some(message) = &mut entry.message {
            message.edited_at = Some(event.edited_at.clone());
            message.content = Some(event.content.clone());
        }
        entry.revision = revision;
        self.dirty.insert(id);
        Ok(true)
    }

    fn apply_reaction(
        &mut self,
        event: &papo_api::gateway::ReactUpdateEvent,
        revision: u64,
    ) -> Result<bool, HistoryError> {
        if event.count < 0 || (event.emoji_id.is_none() == event.unicode.is_none()) {
            return Err(HistoryError::InvalidPage);
        }
        let id = Self::id(&event.message_id)?;
        let entry = self
            .entries
            .entry(id)
            .or_insert_with(|| Entry::empty(revision));
        if entry.deleted {
            return Ok(false);
        }
        let total = MessageReactionSummary {
            emoji_id: event.emoji_id.clone(),
            unicode: event.unicode.clone(),
            count: Some(event.count),
        };
        entry.pending_reactions.retain(|existing| {
            existing.emoji_id != total.emoji_id || existing.unicode != total.unicode
        });
        entry.pending_reactions.push(total.clone());
        if let Some(message) = &mut entry.message {
            replace_total(message.reactions.get_or_insert_with(Vec::new), &total);
        }
        entry.reaction_revision = revision;
        entry.revision = revision;
        self.dirty.insert(id);
        Ok(true)
    }

    pub fn pins(&mut self, ticket: &Ticket, messages: &[Message]) -> Result<(), HistoryError> {
        // The pinned backend serializes pin creation and enforces maxPinnedPerChannel=100.
        if messages.len() > 100 {
            return Err(HistoryError::InvalidPage);
        }
        let mut checked = BTreeSet::new();
        for message in messages {
            if Self::id(
                message
                    .channel_id
                    .as_deref()
                    .ok_or(HistoryError::InvalidPage)?,
            )? != ticket.channel
            {
                return Err(HistoryError::InvalidPage);
            }
            let point = Point::from(&message.cursor().ok_or(HistoryError::InvalidPage)?);
            let key = point.key()?;
            if !checked.insert(key.id)
                || self
                    .entries
                    .get(&key.id)
                    .and_then(|entry| entry.point.as_ref())
                    .is_some_and(|old| old.key().ok() != Some(key))
            {
                return Err(HistoryError::InvalidPage);
            }
            if let Some(edited_at) = &message.edited_at {
                timestamp_us(edited_at)?;
            }
        }
        let revision = self.next_revision()?;
        let mut pinned = BTreeSet::new();
        for message in messages {
            if Self::id(
                message
                    .channel_id
                    .as_deref()
                    .ok_or(HistoryError::InvalidPage)?,
            )? != ticket.channel
            {
                return Err(HistoryError::InvalidPage);
            }
            let point = Point::from(&message.cursor().ok_or(HistoryError::InvalidPage)?);
            let id = point.key()?.id;
            if !pinned.insert(id) {
                return Err(HistoryError::InvalidPage);
            }
            let timeline = self.entries.get(&id).is_some_and(|entry| entry.timeline);
            self.merge_message(message, &point, ticket, revision)?;
            if !timeline {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.timeline = false;
                }
                self.order.remove(&point.key()?);
            }
        }
        for (id, entry) in &mut self.entries {
            if entry.pin_revision <= ticket.revision && !entry.deleted {
                let value = pinned.contains(id);
                if entry.pinned != value {
                    entry.pinned = value;
                    entry.revision = revision;
                    self.dirty.insert(*id);
                }
            }
        }
        Ok(())
    }
}

pub(super) fn replace_total(
    totals: &mut Vec<MessageReactionSummary>,
    total: &MessageReactionSummary,
) {
    totals.retain(|existing| {
        existing.emoji_id != total.emoji_id || existing.unicode != total.unicode
    });
    if total.count.is_some_and(|count| count > 0) {
        totals.push(total.clone());
    }
}
