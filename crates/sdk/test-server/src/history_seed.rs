//! Owns deterministic bulk-history recipes for runtime scenarios; leaves authoritative storage and event delivery outside.

use std::collections::BTreeSet;

use papo_api::{
    Uuid,
    wire::{Channel, Message},
};
use serde::{Deserialize, Serialize};

use crate::{
    Error,
    history::{History, microsecond_instant},
    resources::Resources,
};

/// A deterministic synthetic channel recipe, usable in [`crate::Config::history`].
/// No content, credential or address is embedded; message bodies are absent.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySeed {
    /// UUID of the channel to create (configuration) or populate (public capability).
    pub channel_id: String,
    /// Human author UUID; the recipe does not create an authenticated account.
    pub author_id: String,
    /// First message UUID; subsequent IDs increment its unsigned 128-bit value.
    /// Choose disjoint ranges for multiple recipes; collisions are rejected atomically.
    pub first_message_id: String,
    /// Number of messages, from zero through 100,000 per recipe.
    pub count: usize,
    /// Exact RFC3339 timestamp for the first group, with at most microsecond precision.
    pub first_created_at: String,
    /// Microseconds between timestamp groups; zero gives every message the same instant.
    pub timestamp_step_us: u64,
    /// Messages per equal-timestamp group, at least one.
    pub equal_timestamp_count: usize,
}

impl HistorySeed {
    /// Builds messages in ascending `(created_at,id)` order without clocks or randomness.
    /// IDs increment the explicit first UUID. Reusing a recipe in
    /// one fixture requires deleting its messages first or choosing explicit IDs via `seed_messages`.
    /// The first timestamp is preserved verbatim; later groups use UTC RFC3339 with six fractional digits.
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid UUIDs, timestamp, limits or timestamp overflow.
    pub fn messages(&self) -> Result<Vec<Message>, Error> {
        crate::history::uuid(&self.channel_id)?;
        crate::history::uuid(&self.author_id)?;
        microsecond_instant(&self.first_created_at)?;
        let first_id = crate::history::uuid(&self.first_message_id)?.as_u128();
        if self.count > 100_000 || self.equal_timestamp_count == 0 {
            return Err(Error::Configuration);
        }
        let start = chrono::DateTime::parse_from_rfc3339(&self.first_created_at)
            .map_err(|_| Error::Configuration)?;
        (0..self.count)
            .map(|index| {
                let group = index / self.equal_timestamp_count;
                let offset_us = u64::try_from(group)
                    .ok()
                    .and_then(|group| group.checked_mul(self.timestamp_step_us))
                    .and_then(|offset| i64::try_from(offset).ok())
                    .ok_or(Error::Configuration)?;
                let at = start
                    .checked_add_signed(chrono::Duration::microseconds(offset_us))
                    .ok_or(Error::Configuration)?;
                let created_at = if offset_us == 0 {
                    self.first_created_at.clone()
                } else {
                    at.with_timezone(&chrono::Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
                };
                let id = first_id
                    .checked_add(u128::try_from(index).map_err(|_| Error::Configuration)?)
                    .ok_or(Error::Configuration)?;
                Ok(Message {
                    id: Some(Uuid::from_u128(id).to_string()),
                    channel_id: Some(self.channel_id.clone()),
                    author_id: Some(self.author_id.clone()),
                    created_at: Some(created_at),
                    ..Message::default()
                })
            })
            .collect()
    }
}

pub(crate) fn prepare(seeds: &[HistorySeed]) -> Result<(Resources, History), Error> {
    let mut resources = Resources::default();
    let mut history = History::default();
    for seed in seeds {
        let channel_id = seed.channel_id.clone();
        let position = i64::try_from(resources.channels.len()).map_err(|_| Error::Configuration)?;
        if resources.channels.contains_key(&channel_id) {
            return Err(Error::Configuration);
        }
        resources.channels.insert(
            channel_id.clone(),
            Channel {
                id: Some(channel_id),
                name: Some("history".into()),
                r#type: Some("text".into()),
                position: Some(position),
                permissions: Some(Vec::new()),
                created_at: Some(seed.first_created_at.clone()),
                ..Channel::default()
            },
        );
        insert_batch(&mut history, seed.messages()?)?;
    }
    Ok((resources, history))
}

pub(crate) fn insert_batch(history: &mut History, messages: Vec<Message>) -> Result<(), Error> {
    let mut ids = BTreeSet::<Uuid>::new();
    for message in &messages {
        let (_, key) = crate::history::validate_message(message)?;
        if !ids.insert(key.1) || history.contains(key.1) {
            return Err(Error::Configuration);
        }
    }
    for message in messages {
        history.insert(message)?;
    }
    Ok(())
}
