//! Exact cursor storage and validated microsecond ordering; continuity belongs to the history model.

use papo_api::{HistoryCursor, Uuid};
use serde::{Deserialize, Serialize};

use super::HistoryError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Point {
    pub created_at: String,
    pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub created_at_us: i64,
    pub id: [u8; 16],
}

impl Point {
    pub fn key(&self) -> Result<Key, HistoryError> {
        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|_| HistoryError::InvalidPage)?;
        if timestamp.timestamp_subsec_nanos() % 1_000 != 0 {
            return Err(HistoryError::InvalidPage);
        }
        Ok(Key {
            created_at_us: timestamp.timestamp_micros(),
            id: *Uuid::parse_str(&self.id)
                .map_err(|_| HistoryError::InvalidPage)?
                .as_bytes(),
        })
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

pub(super) fn timestamp_us(value: &str) -> Result<i64, HistoryError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_micros())
        .map_err(|_| HistoryError::InvalidPage)
}

#[cfg(test)]
mod tests {
    //! Precision and tuple-order regressions; transport round-trips are API tests.
    use super::*;

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
