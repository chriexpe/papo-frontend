//! Owns authoritative ordered fixture messages, pins and account-local reactions; leaves HTTP and event delivery outside.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::{
    Uuid,
    wire::{Message, MessageUserReaction},
};

use crate::{Error, history_query::HistoryQuery};

pub(crate) type Key = (i128, Uuid);

#[derive(Default)]
pub(crate) struct History {
    channels: BTreeMap<String, BTreeMap<Key, Message>>,
    index: BTreeMap<Uuid, (String, Key)>,
    pins: BTreeMap<Uuid, i128>,
    own: BTreeMap<(String, Uuid), Vec<MessageUserReaction>>,
    denied: BTreeSet<(String, String)>,
}

impl History {
    pub(crate) fn insert(&mut self, mut message: Message) -> Result<(), Error> {
        let (channel, key) = validate_message(&message)?;
        if self.index.contains_key(&key.1) {
            return Err(Error::Configuration);
        }
        // Ownership is always supplied through an account-scoped capability, never a shared seed.
        message.user_reactions = None;
        message.id = Some(key.1.to_string());
        self.index.insert(key.1, (channel.clone(), key));
        self.channels
            .entry(channel)
            .or_default()
            .insert(key, message);
        Ok(())
    }

    pub(crate) fn latest(&self, channel: &str) -> Option<&Message> {
        self.channels
            .get(channel)?
            .last_key_value()
            .map(|(_, message)| message)
    }

    pub(crate) fn contains(&self, id: Uuid) -> bool {
        self.index.contains_key(&id)
    }

    pub(crate) fn message(&self, id: Uuid) -> Option<&Message> {
        let (channel, key) = self.index.get(&id)?;
        self.channels.get(channel)?.get(key)
    }

    pub(crate) fn message_mut(&mut self, id: Uuid) -> Option<&mut Message> {
        let (channel, key) = self.index.get(&id)?;
        self.channels.get_mut(channel)?.get_mut(key)
    }

    pub(crate) fn remove(&mut self, id: Uuid) -> Option<Message> {
        let (channel, key) = self.index.remove(&id)?;
        self.pins.remove(&id);
        self.own.retain(|(_, message), _| *message != id);
        self.channels.get_mut(&channel)?.remove(&key)
    }

    pub(crate) fn remove_channel(&mut self, channel: &str) {
        let ids: Vec<_> = self
            .channels
            .get(channel)
            .into_iter()
            .flat_map(BTreeMap::keys)
            .map(|key| key.1)
            .collect();
        for id in ids {
            self.remove(id);
        }
        self.channels.remove(channel);
        self.denied.retain(|(_, id)| id != channel);
    }

    pub(crate) fn readable(&self, user: &str, channel: &str) -> bool {
        !self.denied.contains(&(user.to_owned(), channel.to_owned()))
    }

    pub(crate) fn denied_users(&self, channel: &str) -> BTreeSet<String> {
        self.denied
            .iter()
            .filter(|(_, id)| id == channel)
            .map(|(user, _)| user.clone())
            .collect()
    }

    pub(crate) fn set_readable(&mut self, user: &str, channel: &str, readable: bool) {
        let key = (user.to_owned(), channel.to_owned());
        if readable {
            self.denied.remove(&key);
        } else {
            self.denied.insert(key);
        }
    }

    pub(crate) fn page(
        &self,
        channel: &str,
        user: &str,
        query: &HistoryQuery,
    ) -> (Vec<Message>, bool) {
        let Some(messages) = self.channels.get(channel) else {
            return (Vec::new(), false);
        };
        let iter: Box<dyn Iterator<Item = (&Key, &Message)> + '_> = if query.ascending {
            Box::new(messages.iter())
        } else {
            Box::new(messages.iter().rev())
        };
        let mut selected = iter
            .filter(|(key, _)| query.includes(**key))
            .take(query.limit + 1)
            .map(|(key, message)| self.for_user(message, key.1, user))
            .collect::<Vec<_>>();
        let has_more = selected.len() > query.limit;
        selected.truncate(query.limit);
        (selected, has_more)
    }

    pub(crate) fn pinned(&self, channel: &str, user: &str) -> Vec<Message> {
        let mut pins: Vec<_> = self
            .pins
            .iter()
            .filter_map(|(id, at)| {
                let message = self.message(*id)?;
                (message.channel_id.as_deref() == Some(channel)).then_some((*at, *id, message))
            })
            .collect();
        pins.sort_by_key(|(at, id, _)| (*at, *id));
        pins.into_iter()
            .map(|(_, id, message)| self.for_user(message, id, user))
            .collect()
    }

    pub(crate) fn pin(&mut self, id: Uuid, pinned_at: Option<i128>) -> Result<bool, Error> {
        if !self.contains(id) {
            return Err(Error::Configuration);
        }
        match pinned_at {
            Some(at) if !self.pins.contains_key(&id) => {
                let channel = self
                    .message(id)
                    .and_then(|message| message.channel_id.as_deref())
                    .ok_or(Error::Configuration)?;
                if self.pinned(channel, "").len() >= 100 {
                    return Err(Error::Configuration);
                }
                self.pins.insert(id, at);
                Ok(true)
            }
            Some(_) => Ok(false),
            None => Ok(self.pins.remove(&id).is_some()),
        }
    }

    pub(crate) fn set_own(&mut self, user: &str, id: Uuid, reactions: Vec<MessageUserReaction>) {
        self.own.insert((user.to_owned(), id), reactions);
    }

    fn for_user(&self, message: &Message, id: Uuid, user: &str) -> Message {
        let mut message = message.clone();
        message.attachments.get_or_insert_with(Vec::new);
        message.user_reactions = Some(
            self.own
                .get(&(user.to_owned(), id))
                .cloned()
                .unwrap_or_default(),
        );
        message
    }
}

pub(crate) fn uuid(value: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(value).map_err(|_| Error::Configuration)
}

/// Parses an instant without changing the string used in responses or cursors.
pub(crate) fn instant(value: &str) -> Result<i128, Error> {
    // Chrono accepts lowercase separators and leap seconds; Go's RFC3339Nano parser does not.
    if value.as_bytes().get(10) != Some(&b'T')
        || value.ends_with('z')
        || value
            .get(17..19)
            .and_then(|seconds| seconds.parse::<u8>().ok())
            .is_none_or(|seconds| seconds >= 60)
    {
        return Err(Error::Configuration);
    }
    let at = chrono::DateTime::parse_from_rfc3339(value).map_err(|_| Error::Configuration)?;
    Ok(i128::from(at.timestamp()) * 1_000_000_000 + i128::from(at.timestamp_subsec_nanos()))
}

pub(crate) fn microsecond_instant(value: &str) -> Result<i128, Error> {
    let at = instant(value)?;
    if at % 1_000 != 0 {
        return Err(Error::Configuration);
    }
    Ok(at)
}

pub(crate) fn validate_message(message: &Message) -> Result<(String, Key), Error> {
    let id = uuid(message.id.as_deref().ok_or(Error::Configuration)?)?;
    let channel = message.channel_id.as_deref().ok_or(Error::Configuration)?;
    uuid(channel)?;
    if let Some(author) = message.author_id.as_deref() {
        uuid(author)?;
    }
    if let Some(reply) = message.reply_to.as_deref() {
        uuid(reply)?;
    }
    let at = microsecond_instant(message.created_at.as_deref().ok_or(Error::Configuration)?)?;
    if let Some(at) = &message.edited_at {
        microsecond_instant(at)?;
    }
    Ok((channel.to_owned(), (at, id)))
}

#[cfg(test)]
mod tests;
