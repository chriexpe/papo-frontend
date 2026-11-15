//! Exact cursor strings as the server sent them; their parsed order is `message_key::MessageKey`.

use papo_api::HistoryCursor;
use serde::{Deserialize, Serialize};

use super::{HistoryError, message_key::MessageKey};

/// A message boundary kept verbatim so cursors round-trip to the backend unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Point {
    pub created_at: String,
    pub id: String,
}

impl Point {
    pub fn key(&self) -> Result<MessageKey, HistoryError> {
        Ok(MessageKey::parse(&self.created_at, &self.id)?)
    }

    pub fn cursor(&self) -> HistoryCursor {
        HistoryCursor {
            created_at: self.created_at.clone(),
            id: self.id.clone(),
        }
    }
}

impl From<&HistoryCursor> for Point {
    fn from(cursor: &HistoryCursor) -> Self {
        Self {
            created_at: cursor.created_at.clone(),
            id: cursor.id.clone(),
        }
    }
}

/// Edit times follow the same precision rule as creation times.
pub(super) fn timestamp_us(value: &str) -> Result<i64, HistoryError> {
    Ok(super::message_key::parse_message_timestamp_us(value)?)
}

#[cfg(test)]
mod tests {
    //! Cursor text preservation; precision and ordering are `message_key` tests.
    use super::*;
    use papo_api::Uuid;

    #[test]
    fn equal_microseconds_order_by_uuid_without_changing_cursor_text() {
        let first = Point {
            created_at: "2026-01-01T03:00:00.123456+03:00".into(),
            id: Uuid::from_u128(1).to_string(),
        };
        let second = Point {
            created_at: "2026-01-01T00:00:00.123456Z".into(),
            id: Uuid::from_u128(2).to_string(),
        };
        assert!(first.key().unwrap() < second.key().unwrap());
        assert_eq!(first.cursor().created_at, first.created_at);
    }

    #[test]
    fn submicrosecond_input_is_not_silently_rounded_into_continuity() {
        let point = Point {
            created_at: "2026-01-01T00:00:00.123456789Z".into(),
            id: Uuid::nil().to_string(),
        };
        assert!(point.key().is_err());
    }
}
