//! Canonical chronological message ordering (`created_at_us`, `message_id`) and cursor parsing.
//! History coverage, reading cursors, detached search and notification cursors share this key.

use papo_api::{HistoryCursor, Uuid, wire::Message};

/// Failure parsing or formatting a chronological cursor key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ChronologyError {
    /// Timestamp is missing, malformed or carries sub-microsecond precision.
    #[error("invalid microsecond timestamp")]
    InvalidTimestamp,
    /// Message UUID is missing or malformed.
    #[error("invalid message UUID")]
    InvalidId,
    /// Timestamp microseconds fall outside representable UTC range.
    #[error("timestamp outside UTC range")]
    OutOfRange,
}

/// Canonical message-order key, never a server acknowledgement timestamp.
/// Ordering is PostgreSQL microseconds since Unix epoch followed by the
/// opaque UUID's big-endian byte representation, matching history pagination.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ChronologicalKey {
    /// Server message creation time in microseconds since Unix epoch.
    pub created_at_us: i64,
    /// Backend message UUID, used only as an equal-timestamp tiebreaker.
    #[serde(
        serialize_with = "crate::read_points::serialize_id",
        deserialize_with = "crate::read_points::deserialize_id"
    )]
    pub message_id: Uuid,
}

impl ChronologicalKey {
    /// Parses an RFC 3339 message timestamp into Unix microseconds,
    /// rejecting timestamps with sub-microsecond digits.
    ///
    /// # Errors
    /// Returns `ChronologyError::InvalidTimestamp` when `value` is not valid
    /// RFC 3339 or contains non-zero sub-microsecond digits.
    pub fn parse_timestamp_us(value: &str) -> Result<i64, ChronologyError> {
        let parsed = chrono::DateTime::parse_from_rfc3339(value)
            .map_err(|_| ChronologyError::InvalidTimestamp)?;
        if parsed.timestamp_subsec_nanos() % 1_000 != 0 {
            return Err(ChronologyError::InvalidTimestamp);
        }
        Ok(parsed.timestamp_micros())
    }

    /// Formats Unix microseconds as canonical microsecond UTC RFC 3339 text.
    ///
    /// # Errors
    /// Returns `ChronologyError::OutOfRange` when `created_at_us` falls outside UTC range.
    pub fn format_timestamp_us(created_at_us: i64) -> Result<String, ChronologyError> {
        Ok(chrono::DateTime::from_timestamp_micros(created_at_us)
            .ok_or(ChronologyError::OutOfRange)?
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
    }

    /// Parses optional cursor strings into a canonical microsecond key.
    ///
    /// # Errors
    /// Returns `ChronologyError::InvalidTimestamp` when `created_at` is missing or
    /// invalid (including sub-microsecond digits), and `ChronologyError::InvalidId`
    /// when `id` is missing or not a valid UUID.
    pub fn parse(created_at: Option<&str>, id: Option<&str>) -> Result<Self, ChronologyError> {
        let created_at_us =
            Self::parse_timestamp_us(created_at.ok_or(ChronologyError::InvalidTimestamp)?)?;
        let message_id = id
            .ok_or(ChronologyError::InvalidId)?
            .parse()
            .map_err(|_| ChronologyError::InvalidId)?;
        Ok(Self {
            created_at_us,
            message_id,
        })
    }

    /// Parses the canonical chronological cursor from a `Message`.
    ///
    /// # Errors
    /// Returns a `ChronologyError` when `created_at` or `id` is absent or invalid.
    pub fn from_message(message: &Message) -> Result<Self, ChronologyError> {
        Self::parse(message.created_at.as_deref(), message.id.as_deref())
    }

    /// Parses a wire `HistoryCursor` into a canonical chronological key.
    ///
    /// # Errors
    /// Returns a `ChronologyError` when `created_at` or `id` is invalid.
    pub fn from_cursor(cursor: &HistoryCursor) -> Result<Self, ChronologyError> {
        Self::parse(Some(&cursor.created_at), Some(&cursor.id))
    }

    /// Formats this key as a microsecond RFC 3339 `HistoryCursor`.
    ///
    /// # Errors
    /// Returns `ChronologyError::OutOfRange` when `created_at_us` falls outside UTC range.
    pub fn to_cursor(&self) -> Result<HistoryCursor, ChronologyError> {
        Ok(HistoryCursor {
            created_at: Self::format_timestamp_us(self.created_at_us)?,
            id: self.message_id.to_string(),
        })
    }

    /// Returns the big-endian UUID bytes used for equal-timestamp tiebreaking.
    #[must_use]
    pub fn id_bytes(self) -> [u8; 16] {
        *self.message_id.as_bytes()
    }
}

/// Returns `true` when `message` was authored by a user other than `self_user`
/// (including unparsed or absent author metadata).
#[must_use]
pub fn is_foreign_message(message: &Message, self_user: Uuid) -> bool {
    message
        .author_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        != Some(self_user)
}

#[cfg(test)]
mod tests {
    //! Canonical key ordering, strict sub-microsecond rejection and author predicate checks.
    use super::*;

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn equal_microseconds_order_by_uuid_bytes_and_round_trip_cursor() {
        let lower = ChronologicalKey::parse(
            Some("2026-04-01T12:00:00.000123Z"),
            Some("00000000-0000-0000-0000-000000000001"),
        )
        .unwrap();
        let higher = ChronologicalKey::parse(
            Some("2026-04-01T12:00:00.000123Z"),
            Some("00000000-0000-0000-0000-000000000002"),
        )
        .unwrap();
        assert!(lower < higher);
        assert!(lower.id_bytes() < higher.id_bytes());
        let cursor = lower.to_cursor().unwrap();
        assert_eq!(ChronologicalKey::from_cursor(&cursor).unwrap(), lower);
    }

    #[test]
    fn sub_microsecond_digits_are_rejected_and_whole_micros_accepted() {
        let err = ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00.1234567Z");
        assert_eq!(err, Err(ChronologyError::InvalidTimestamp));

        let err = ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00.000000001Z");
        assert_eq!(err, Err(ChronologyError::InvalidTimestamp));

        assert!(ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00Z").is_ok());
        assert!(ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00.123Z").is_ok());
        assert!(ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00.123456Z").is_ok());
        assert!(ChronologicalKey::parse_timestamp_us("2026-04-01T12:00:00.123456000Z").is_ok());
    }

    #[test]
    fn foreign_message_checks_author_id() {
        let self_id = uuid(1);
        let other_id = uuid(2);
        let mut msg = Message {
            author_id: Some(self_id.to_string()),
            ..Message::default()
        };
        assert!(!is_foreign_message(&msg, self_id));
        msg.author_id = Some(other_id.to_string());
        assert!(is_foreign_message(&msg, self_id));
        msg.author_id = Some("not-a-uuid".into());
        assert!(is_foreign_message(&msg, self_id));
        msg.author_id = None;
        assert!(is_foreign_message(&msg, self_id));
    }
}
