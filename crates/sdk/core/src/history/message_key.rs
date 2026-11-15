//! The one chronological message key and its parsing rule; exact cursor strings live in `ordering`.
//!
//! History, reading, timeline windows, search and the notification inbox all order messages by
//! `(created_at_us, id)`. They share this type and this parser, so a timestamp that one of them
//! accepts is accepted by all of them, and none of them silently rounds.

use papo_api::{HistoryCursor, Uuid, wire::Message};

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
}
