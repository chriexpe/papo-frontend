//! Owns pinned channel read acknowledgements and silent other-device advances; leaves upload/debounce policy outside.

use papo_api::{Uuid, wire::ReadChannelRequest};

use crate::{
    Error, TestServer,
    history::{microsecond_instant, validate_message},
    response::{Reply, problem},
    server::State,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadFailure {
    InvalidInput,
    Missing,
    Forbidden,
    Internal,
}

impl ReadFailure {
    fn reply(self, path: &str) -> Reply {
        let (status, slug) = match self {
            Self::InvalidInput => (400, "invalid-param"),
            Self::Missing => (404, "not-found"),
            Self::Forbidden => (403, "forbidden"),
            Self::Internal => (500, "internal"),
        };
        problem(status, slug, path)
    }
}

pub(crate) fn channel_id(path: &str) -> Option<&str> {
    path.strip_prefix("/channels/")?
        .strip_suffix("/read")
        .filter(|id| !id.contains('/'))
}

pub(crate) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    user: Option<&str>,
    body: &[u8],
) -> Option<Reply> {
    let channel = channel_id(path).filter(|_| method == "PUT")?;
    let Some(user) = user else {
        return Some(problem(401, "unauthorized", path));
    };
    let result = serde_json::from_slice::<ReadChannelRequest>(body)
        .map_err(|_| ReadFailure::InvalidInput)
        .and_then(|request| {
            let at = state.read_acknowledged_at.clone();
            advance(
                &mut state.resources,
                &state.history,
                user,
                channel,
                &request.message_id,
                &at,
            )
        });
    Some(match result {
        Ok(_) => Reply::empty(),
        Err(error) => error.reply(path),
    })
}

fn advance(
    resources: &mut crate::resources::Resources,
    history: &crate::history::History,
    user: &str,
    channel: &str,
    message: &str,
    acknowledged_at: &str,
) -> Result<bool, ReadFailure> {
    if user.is_empty() || channel.is_empty() || message.is_empty() {
        return Err(ReadFailure::InvalidInput);
    }
    // The pinned Go service passes malformed UUIDs to PostgreSQL, which yields an internal error.
    Uuid::parse_str(channel).map_err(|_| ReadFailure::Internal)?;
    if !resources.channels.contains_key(channel) {
        return Err(ReadFailure::Missing);
    }
    if !history.readable(user, channel) {
        return Err(ReadFailure::Forbidden);
    }
    let id = Uuid::parse_str(message).map_err(|_| ReadFailure::Internal)?;
    let target = history.message(id).ok_or(ReadFailure::Missing)?;
    let (target_channel, target_key) =
        validate_message(target).map_err(|_| ReadFailure::Internal)?;
    if target_channel != channel {
        return Err(ReadFailure::Missing);
    }
    let account_channel = (user.to_owned(), channel.to_owned());
    if let Some((previous_id, _)) = resources.read_marks.get(&account_channel) {
        let previous_key = Uuid::parse_str(previous_id)
            .ok()
            .and_then(|id| history.message(id))
            .and_then(|message| validate_message(message).ok())
            .map_or((0, Uuid::nil()), |(_, key)| key);
        if target_key <= previous_key {
            return Ok(false);
        }
    }
    resources.read_marks.insert(
        account_channel,
        (id.to_string(), acknowledged_at.to_owned()),
    );
    Ok(true)
}

impl TestServer {
    /// Sets the synthetic acknowledgement time used by subsequent HTTP PUT reads.
    /// Defaults to the fixture's fixed creation timestamp; never reads a real clock.
    /// This is acknowledgement wall time, not the message-order cursor.
    /// # Errors
    /// Returns [`Error::Configuration`] unless the timestamp is RFC3339 with microsecond precision or less.
    pub async fn set_read_acknowledged_at(&self, acknowledged_at: &str) -> Result<(), Error> {
        microsecond_instant(acknowledged_at)?;
        self.state.lock().await.read_acknowledged_at = acknowledged_at.to_owned();
        Ok(())
    }

    /// Simulates another device's explicit read using the pinned strict `(created_at,id)` comparison.
    /// Returns true only when the mark changes; equal/older requests preserve its acknowledgement time.
    /// GET /channels reveals the change only on refresh/reconnect; no gateway event is invented.
    /// A deleted previous message compares as `(Unix epoch, nil UUID)`, matching the pinned SQL.
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid timestamps, inaccessible/missing channels or messages.
    pub async fn advance_read_mark(
        &self,
        user_id: &str,
        channel_id: &str,
        message_id: &str,
        acknowledged_at: &str,
    ) -> Result<bool, Error> {
        microsecond_instant(acknowledged_at)?;
        let mut state = self.state.lock().await;
        let state = &mut *state;
        advance(
            &mut state.resources,
            &state.history,
            user_id,
            channel_id,
            message_id,
            acknowledged_at,
        )
        .map_err(|_| Error::Configuration)
    }
}

#[cfg(test)]
mod tests;
