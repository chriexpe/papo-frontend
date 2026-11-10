//! Owns public history fixture capabilities and backend-shaped live events; leaves REST dispatch and SDK merge policy outside.

use papo_api::{
    gateway::{
        KnownEvent, MessageDeleteEvent, MessageEditEvent, MessageEmbedsUpdateEvent, MessageEvent,
        MessagePinEvent, ReactUpdateEvent,
    },
    wire::{Embed, Message, MessageReactionSummary, MessageUserReaction},
};

use crate::{
    Error, HistorySeed, TestServer,
    history::{microsecond_instant, uuid},
    history_seed::insert_batch,
    server::State,
};

impl TestServer {
    /// Atomically adds a silent batch to an existing channel without replaying events.
    /// `created_at` strings round-trip exactly; shared `user_reactions` are discarded.
    /// Use [`Self::set_user_reactions`] for account-local ownership. Deleted IDs can be
    /// reintroduced deliberately by a later script; no REST tombstone/resume is invented.
    /// # Errors
    /// Returns [`Error::Configuration`] for missing channels, mismatched channel IDs,
    /// duplicate/global-existing IDs, invalid UUIDs or non-microsecond timestamps.
    pub async fn seed_messages(
        &self,
        channel_id: &str,
        messages: Vec<Message>,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        if !state.resources.channels.contains_key(channel_id)
            || messages
                .iter()
                .any(|message| message.channel_id.as_deref() != Some(channel_id))
        {
            return Err(Error::Configuration);
        }
        insert_batch(&mut state.history, messages)
    }

    /// Adds a deterministic bulk recipe to an existing channel, without events.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid recipe or batch; see [`Self::seed_messages`].
    pub async fn seed_history(&self, seed: HistorySeed) -> Result<(), Error> {
        let messages = seed.messages()?;
        self.seed_messages(&seed.channel_id, messages).await
    }

    /// Adds one authoritative message and emits the backend's `message` event to readers.
    /// Existing embeds follow in a separate `message_embeds_update`, as in the backend.
    /// Reaction summaries and own reactions are not invented in either event.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid/duplicate message or missing channel.
    pub async fn publish_message(&self, message: Message) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        let channel = message
            .channel_id
            .as_deref()
            .ok_or(Error::Configuration)?
            .to_owned();
        if !state.resources.channels.contains_key(&channel) {
            return Err(Error::Configuration);
        }
        let id = uuid(message.id.as_deref().ok_or(Error::Configuration)?)?.to_string();
        let embeds = message.embeds.clone().unwrap_or_default();
        let event = MessageEvent {
            id: id.clone(),
            channel_id: channel.clone(),
            author_id: message.author_id.clone().unwrap_or_default(),
            webhook_id: message.webhook_id.clone(),
            external_author: message.external_author.clone(),
            content: message.content.clone().unwrap_or_default(),
            created_at: message.created_at.clone().ok_or(Error::Configuration)?,
            reply_to: message.reply_to.clone(),
            attachments: message.attachments.clone().unwrap_or_default(),
        };
        state.history.insert(message)?;
        emit_channel(&state, &channel, KnownEvent::Message(Box::new(event)))?;
        if !embeds.is_empty() {
            emit_channel(
                &state,
                &channel,
                KnownEvent::MessageEmbedsUpdate(Box::new(MessageEmbedsUpdateEvent {
                    channel_id: channel.clone(),
                    message_id: id,
                    embeds,
                })),
            )?;
        }
        Ok(())
    }

    /// Replaces authoritative content and `edited_at`; emits `message_edit` followed by
    /// the current `message_embeds_update`, as the pinned edit handler does.
    /// Explicit timestamps allow out-of-order/duplicate scripts without a real clock.
    /// # Errors
    /// Returns [`Error::Configuration`] for missing messages or non-microsecond timestamps.
    pub async fn edit_message(
        &self,
        message_id: &str,
        content: &str,
        edited_at: &str,
    ) -> Result<(), Error> {
        microsecond_instant(edited_at)?;
        let id = uuid(message_id)?;
        let mut state = self.state.lock().await;
        let message = state.history.message_mut(id).ok_or(Error::Configuration)?;
        let channel = message.channel_id.clone().ok_or(Error::Configuration)?;
        message.content = Some(content.to_owned());
        message.edited_at = Some(edited_at.to_owned());
        let embeds = message.embeds.clone().unwrap_or_default();
        emit_channel(
            &state,
            &channel,
            KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                id: id.to_string(),
                channel_id: channel.clone(),
                content: content.to_owned(),
                edited_at: edited_at.to_owned(),
            })),
        )?;
        emit_channel(
            &state,
            &channel,
            KnownEvent::MessageEmbedsUpdate(Box::new(MessageEmbedsUpdateEvent {
                channel_id: channel.clone(),
                message_id: id.to_string(),
                embeds,
            })),
        )
    }

    /// Removes a message, its pin and all own-reaction entries, and emits `message_delete`.
    /// Returns false for an already absent UUID; no REST deletion tombstone is exposed.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid UUID or event encoding failure.
    pub async fn delete_message(&self, message_id: &str) -> Result<bool, Error> {
        let id = uuid(message_id)?;
        let mut state = self.state.lock().await;
        let Some(message) = state.history.remove(id) else {
            return Ok(false);
        };
        let channel = message.channel_id.ok_or(Error::Configuration)?;
        emit_channel(
            &state,
            &channel,
            KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: id.to_string(),
                channel_id: channel.clone(),
            })),
        )?;
        Ok(true)
    }

    /// Replaces embeds and emits the backend's complete `message_embeds_update` list.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid or missing message UUID.
    pub async fn set_message_embeds(
        &self,
        message_id: &str,
        embeds: Vec<Embed>,
    ) -> Result<(), Error> {
        let id = uuid(message_id)?;
        let mut state = self.state.lock().await;
        let message = state.history.message_mut(id).ok_or(Error::Configuration)?;
        message.embeds = Some(embeds.clone());
        let channel = message.channel_id.clone().ok_or(Error::Configuration)?;
        emit_channel(
            &state,
            &channel,
            KnownEvent::MessageEmbedsUpdate(Box::new(MessageEmbedsUpdateEvent {
                channel_id: channel.clone(),
                message_id: id.to_string(),
                embeds,
            })),
        )
    }

    /// Replaces one emoji's server total and emits `react_update`; zero removes it.
    /// Ownership is independent and account-scoped via [`Self::set_user_reactions`].
    /// # Errors
    /// Returns [`Error::Configuration`] for a missing message, invalid selector or negative/missing total.
    pub async fn set_message_reaction(
        &self,
        message_id: &str,
        reaction: MessageReactionSummary,
    ) -> Result<(), Error> {
        validate_selector(reaction.emoji_id.as_deref(), reaction.unicode.as_deref())?;
        let count = reaction
            .count
            .filter(|count| *count >= 0)
            .ok_or(Error::Configuration)?;
        let id = uuid(message_id)?;
        let mut state = self.state.lock().await;
        let message = state.history.message_mut(id).ok_or(Error::Configuration)?;
        let channel = message.channel_id.clone().ok_or(Error::Configuration)?;
        let reactions = message.reactions.get_or_insert_with(Vec::new);
        reactions.retain(|current| {
            (current.emoji_id.as_ref(), current.unicode.as_ref())
                != (reaction.emoji_id.as_ref(), reaction.unicode.as_ref())
        });
        if count > 0 {
            reactions.push(reaction.clone());
        }
        emit_channel(
            &state,
            &channel,
            KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
                message_id: id.to_string(),
                emoji_id: reaction.emoji_id,
                unicode: reaction.unicode,
                count,
            })),
        )
    }

    /// Replaces one account's own reactions in history and pinned payloads, silently.
    /// The backend reaction event does not carry ownership. Scripts set server totals
    /// separately, allowing stale events to be emitted with [`Self::emit_event`].
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid account/message/reaction UUIDs, selectors or a missing message.
    pub async fn set_user_reactions(
        &self,
        user_id: &str,
        message_id: &str,
        reactions: Vec<MessageUserReaction>,
    ) -> Result<(), Error> {
        uuid(user_id)?;
        let id = uuid(message_id)?;
        for reaction in &reactions {
            uuid(reaction.id.as_deref().ok_or(Error::Configuration)?)?;
            validate_selector(reaction.emoji_id.as_deref(), reaction.unicode.as_deref())?;
        }
        let mut state = self.state.lock().await;
        if !state.history.contains(id) {
            return Err(Error::Configuration);
        }
        state.history.set_own(user_id, id, reactions);
        Ok(())
    }

    /// Pins at an explicit microsecond timestamp, or unpins with None. Existing pins
    /// are idempotent and keep their original order. No pin metadata is added to messages.
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid/missing messages, timestamps or the backend's 100-pin cap.
    pub async fn set_message_pin(
        &self,
        message_id: &str,
        pinned_at: Option<&str>,
    ) -> Result<(), Error> {
        let at = pinned_at.map(microsecond_instant).transpose()?;
        let id = uuid(message_id)?;
        let mut state = self.state.lock().await;
        let channel = state
            .history
            .message(id)
            .and_then(|message| message.channel_id.clone())
            .ok_or(Error::Configuration)?;
        if state.history.pin(id, at)? {
            emit_channel(
                &state,
                &channel,
                KnownEvent::MessagePin(Box::new(MessagePinEvent {
                    message_id: id.to_string(),
                    is_pinned: at.is_some(),
                })),
            )?;
        }
        Ok(())
    }

    /// Scripts the backend's read-authorization result for one account/channel.
    /// Default is readable; this is not a role/DM permission engine. Denial affects
    /// history, pinned lists and these controls' live events, not channel-list visibility.
    /// The pin has no access-revocation event; reconnect/refetch discovers HTTP 403.
    /// # Errors
    /// Returns [`Error::Configuration`] for an invalid account UUID or missing channel.
    pub async fn set_channel_readable(
        &self,
        user_id: &str,
        channel_id: &str,
        readable: bool,
    ) -> Result<(), Error> {
        uuid(user_id)?;
        let mut state = self.state.lock().await;
        if !state.resources.channels.contains_key(channel_id) {
            return Err(Error::Configuration);
        }
        state.history.set_readable(user_id, channel_id, readable);
        Ok(())
    }
}

fn validate_selector(emoji_id: Option<&str>, unicode: Option<&str>) -> Result<(), Error> {
    match (emoji_id, unicode) {
        (Some(id), None) => {
            uuid(id)?;
            Ok(())
        }
        (None, Some(value)) if !value.is_empty() => Ok(()),
        _ => Err(Error::Configuration),
    }
}

pub(crate) fn emit_channel(state: &State, channel: &str, event: KnownEvent) -> Result<(), Error> {
    let mut payload = serde_json::to_value(event).map_err(|_| Error::Configuration)?;
    if payload["type"] == "message" {
        for key in ["webhook_id", "external_author", "attachments"] {
            if payload[key].is_null() || payload[key].as_array().is_some_and(Vec::is_empty) {
                payload
                    .as_object_mut()
                    .ok_or(Error::Configuration)?
                    .remove(key);
            }
        }
    }
    state
        .presence
        .emit_except(state.history.denied_users(channel), payload);
    Ok(())
}
