//! The one chronological message key and its parsing rule; exact cursor strings live in `ordering`.
//!
//! History, reading, timeline windows, search and the notification inbox all order messages by
//! `(created_at_us, id)`. They share this type and this parser, so a timestamp that one of them
//! accepts is accepted by all of them, and none of them silently rounds.

use papo_api::{
    HistoryCursor, Uuid,
    wire::{Message, MessageListResponse},
};

/// Message order: PostgreSQL microseconds, then the opaque UUID's bytes, exactly like the
/// backend's `(created_at, id)` cursor comparison. Never an acknowledgement timestamp.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct MessageKey {
    /// Server message creation time in microseconds since the Unix epoch.
    pub created_at_us: i64,
    /// Backend message UUID, used only as an equal-timestamp tiebreaker.
    #[serde(
        serialize_with = "crate::read_points::serialize_id",
        deserialize_with = "crate::read_points::deserialize_id"
    )]
    pub message_id: Uuid,
}

/// A timestamp or ID that cannot be a message key: missing, not RFC 3339, finer than one
/// microsecond, or not a UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid message key")]
pub struct InvalidMessageKey;

impl MessageKey {
    /// Parses a key from its exact server fields.
    /// # Errors
    /// Returns `InvalidMessageKey` for a non-RFC 3339 or sub-microsecond timestamp or a non-UUID ID.
    pub fn parse(created_at: &str, id: &str) -> Result<Self, InvalidMessageKey> {
        Ok(Self {
            created_at_us: parse_message_timestamp_us(created_at)?,
            message_id: id.parse().map_err(|_| InvalidMessageKey)?,
        })
    }

    /// Parses the key of a canonical history message.
    /// # Errors
    /// Returns `InvalidMessageKey` when `created_at` or `id` is absent or invalid.
    pub fn from_message(message: &Message) -> Result<Self, InvalidMessageKey> {
        Self::parse(
            message.created_at.as_deref().ok_or(InvalidMessageKey)?,
            message.id.as_deref().ok_or(InvalidMessageKey)?,
        )
    }

    /// Parses an exact wire history cursor.
    /// # Errors
    /// Returns `InvalidMessageKey` when the cursor is not a valid message key.
    pub fn from_cursor(cursor: &HistoryCursor) -> Result<Self, InvalidMessageKey> {
        Self::parse(&cursor.created_at, &cursor.id)
    }

    /// Formats this key as a microsecond RFC 3339 cursor. Prefer a held message's exact cursor;
    /// this is for keys whose original server text is no longer available.
    /// # Errors
    /// Returns `InvalidMessageKey` if `created_at_us` is outside the representable UTC range.
    pub fn to_cursor(&self) -> Result<HistoryCursor, InvalidMessageKey> {
        let created_at = chrono::DateTime::from_timestamp_micros(self.created_at_us)
            .ok_or(InvalidMessageKey)?
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        Ok(HistoryCursor {
            created_at,
            id: self.message_id.to_string(),
        })
    }

    /// The ID as the history model's map key.
    pub(crate) fn id_bytes(self) -> [u8; 16] {
        *self.message_id.as_bytes()
    }
}

/// The one precision rule for message timestamps (creation and edit times): RFC 3339 with at
/// most microsecond precision. Sub-microsecond digits are rejected, never rounded, because a
/// rounded key could reorder two messages the backend keeps distinct.
/// # Errors
/// Returns `InvalidMessageKey` for anything else.
pub fn parse_message_timestamp_us(raw: &str) -> Result<i64, InvalidMessageKey> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| InvalidMessageKey)?;
    if timestamp.timestamp_subsec_nanos() % 1_000 != 0 {
        return Err(InvalidMessageKey);
    }
    Ok(timestamp.timestamp_micros())
}

/// Validated keys and exact continuation cursor from one descending message page.
/// Used by detached point lookups (reading and notifications); never grants history coverage.
pub(crate) struct ValidatedDescendingPage {
    pub points: Vec<MessageKey>,
    pub next_cursor: Option<HistoryCursor>,
}

/// Validates one bounded descending `GET /channels/{id}/messages` response: matching channel,
/// explicit `messages` and `has_more`, at most `limit` rows, strictly descending `MessageKey`
/// order below `before`, and an exact continuation cursor whenever `has_more` is true.
/// # Errors
/// Returns `InvalidMessageKey` for any missing required field, channel mismatch, bound violation,
/// malformed key or non-descending order.
pub(crate) fn validate_descending_page(
    channel: Uuid,
    before: Option<&HistoryCursor>,
    limit: usize,
    page: &MessageListResponse,
) -> Result<ValidatedDescendingPage, InvalidMessageKey> {
    if page
        .channel_id
        .as_deref()
        .and_then(|id| id.parse::<Uuid>().ok())
        != Some(channel)
    {
        return Err(InvalidMessageKey);
    }
    let messages = page.messages.as_deref().ok_or(InvalidMessageKey)?;
    let has_more = page.has_more.ok_or(InvalidMessageKey)?;
    if messages.len() > limit {
        return Err(InvalidMessageKey);
    }
    let mut previous = before.map(MessageKey::from_cursor).transpose()?;
    let mut points = Vec::with_capacity(messages.len());
    for message in messages {
        if message
            .channel_id
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
            != Some(channel)
        {
            return Err(InvalidMessageKey);
        }
        let point = MessageKey::from_message(message)?;
        if previous.is_some_and(|previous| point >= previous) {
            return Err(InvalidMessageKey);
        }
        previous = Some(point);
        points.push(point);
    }
    let next_cursor = if has_more {
        Some(
            messages
                .last()
                .and_then(Message::cursor)
                .ok_or(InvalidMessageKey)?,
        )
    } else {
        None
    };
    Ok(ValidatedDescendingPage {
        points,
        next_cursor,
    })
}

#[cfg(test)]
mod tests {
    //! Precision and tuple-order regressions; transport round-trips are API tests.
    use super::*;

    #[test]
    fn equal_microseconds_order_by_uuid_across_offsets() {
        let first = MessageKey::parse(
            "2026-01-01T03:00:00.123456+03:00",
            &Uuid::from_u128(1).to_string(),
        )
        .unwrap();
        let second = MessageKey::parse(
            "2026-01-01T00:00:00.123456Z",
            &Uuid::from_u128(2).to_string(),
        )
        .unwrap();
        assert_eq!(first.created_at_us, second.created_at_us);
        assert!(first < second);
    }

    #[test]
    fn submicrosecond_digits_are_rejected_not_rounded() {
        let id = Uuid::nil().to_string();
        assert_eq!(
            MessageKey::parse("2026-01-01T00:00:00.123456789Z", &id),
            Err(InvalidMessageKey)
        );
        assert_eq!(
            MessageKey::parse("2026-01-01T00:00:00.123456000Z", &id)
                .unwrap()
                .created_at_us
                % 1_000_000,
            123_456
        );
    }

    #[test]
    fn cursor_round_trip_keeps_the_key() {
        let key =
            MessageKey::parse("2026-01-01T00:00:00.000001Z", &Uuid::max().to_string()).unwrap();
        assert_eq!(MessageKey::from_cursor(&key.to_cursor().unwrap()), Ok(key));
    }

    fn message(channel: Uuid, id: u128, created_at: &str) -> Message {
        Message {
            id: Some(Uuid::from_u128(id).to_string()),
            channel_id: Some(channel.to_string()),
            created_at: Some(created_at.into()),
            ..Message::default()
        }
    }

    #[test]
    fn validate_descending_page_accepts_valid_pages_and_rejects_corruption() {
        let channel = Uuid::from_u128(10);
        let other = Uuid::from_u128(11);
        let newer = message(channel, 2, "2026-01-01T00:00:02.000000Z");
        let older = message(channel, 1, "2026-01-01T00:00:01.000000Z");
        let valid = MessageListResponse {
            channel_id: Some(channel.to_string()),
            messages: Some(vec![newer.clone(), older.clone()]),
            has_more: Some(true),
        };
        let validated = validate_descending_page(channel, None, 2, &valid).unwrap();
        assert_eq!(validated.points.len(), 2);
        assert!(validated.next_cursor == older.cursor());

        // Missing messages or has_more is corrupt data, never an empty terminal page.
        let missing_messages = MessageListResponse {
            messages: None,
            ..valid.clone()
        };
        assert!(validate_descending_page(channel, None, 2, &missing_messages).is_err());
        let missing_has_more = MessageListResponse {
            has_more: None,
            ..valid.clone()
        };
        assert!(validate_descending_page(channel, None, 2, &missing_has_more).is_err());

        // Wrong channel, exceeding limit, non-descending order, cursor non-progress and
        // has_more without a cursor row are all rejected.
        let wrong_channel = MessageListResponse {
            channel_id: Some(other.to_string()),
            ..valid.clone()
        };
        assert!(validate_descending_page(channel, None, 2, &wrong_channel).is_err());
        let wrong_row_channel = MessageListResponse {
            messages: Some(vec![message(other, 2, "2026-01-01T00:00:02.000000Z")]),
            has_more: Some(false),
            ..valid.clone()
        };
        assert!(validate_descending_page(channel, None, 2, &wrong_row_channel).is_err());
        assert!(validate_descending_page(channel, None, 1, &valid).is_err());
        let ascending = MessageListResponse {
            messages: Some(vec![older.clone(), newer.clone()]),
            has_more: Some(false),
            ..valid.clone()
        };
        assert!(validate_descending_page(channel, None, 2, &ascending).is_err());
        assert!(validate_descending_page(channel, newer.cursor().as_ref(), 2, &valid).is_err());
        let empty_with_more = MessageListResponse {
            messages: Some(Vec::new()),
            has_more: Some(true),
            ..valid
        };
        assert!(validate_descending_page(channel, None, 2, &empty_with_more).is_err());
    }
}
