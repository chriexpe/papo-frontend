//! Live message mutation precedence and detached pins; paging and socket journaling live elsewhere.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::{
    gateway::KnownEvent,
    wire::{Message, MessageReactionSummary},
};

use serde::{Deserialize, Serialize};

use super::{
    HistoryError,
    merge::Ticket,
    model::{Entry, Model},
    ordering::{Point, timestamp_us},
    pins::PinRank,
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
                entry.own_reactions.clear();
                entry.revision = revision;
                self.dirty.insert(id);
                self.pin_ranks.remove(&id);
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
                if event.is_pinned {
                    self.pin_ranks.insert(
                        id,
                        PinRank {
                            revision,
                            position: 0,
                        },
                    );
                } else {
                    self.pin_ranks.remove(&id);
                }
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
            existing.total.emoji_id != total.emoji_id || existing.total.unicode != total.unicode
        });
        if let Some(message) = &mut entry.message {
            replace_total(message.reactions.get_or_insert_with(Vec::new), &total);
        }
        entry
            .pending_reactions
            .push(PendingTotal { total, revision });
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
            if !checked.insert(key.id_bytes())
                || self
                    .entries
                    .get(&key.id_bytes())
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
        // The backend lists pins oldest pin first.
        let mut pinned = BTreeMap::new();
        for (position, message) in messages.iter().enumerate() {
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
            let id = point.key()?.id_bytes();
            let rank = PinRank {
                revision: ticket.revision,
                position: u32::try_from(position).map_err(|_| HistoryError::InvalidPage)?,
            };
            if pinned.insert(id, rank).is_some() {
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
                let rank = pinned.get(id).copied();
                if entry.pinned != rank.is_some() {
                    entry.pinned = rank.is_some();
                    entry.revision = revision;
                    self.dirty.insert(*id);
                }
                match rank {
                    Some(rank) => self.pin_ranks.insert(*id, rank),
                    None => self.pin_ranks.remove(id),
                };
            }
        }
        self.pins_generation = ticket.generation;
        Ok(())
    }
}

/// A live total and the model revision it arrived at: a page asked for before that revision
/// cannot hold it.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PendingTotal {
    #[serde(flatten)]
    pub total: MessageReactionSummary,
    /// 0 for a total stored before revisions were kept: any page replaces it.
    #[serde(default)]
    pub revision: u64,
}

/// Sets one emoji's total. An emoji keeps its place (the backend lists them by first
/// reaction), a new one goes last, and one whose count reaches zero is gone.
pub(super) fn replace_total(
    totals: &mut Vec<MessageReactionSummary>,
    total: &MessageReactionSummary,
) {
    let present = total.count.is_some_and(|count| count > 0);
    let held = totals.iter().position(|existing| {
        existing.emoji_id == total.emoji_id && existing.unicode == total.unicode
    });
    match (held, present) {
        (Some(index), true) => totals[index].count = total.count,
        (Some(index), false) => {
            totals.remove(index);
        }
        (None, true) => totals.push(total.clone()),
        (None, false) => {}
    }
}
