//! Exact cursor storage and validated microsecond ordering; continuity belongs to the history model.

use papo_api::HistoryCursor;
use serde::{Deserialize, Serialize};

use super::HistoryError;

pub(super) use crate::ChronologicalKey as Key;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Point {
    pub created_at: String,
    pub id: String,
}

impl Point {
    pub fn key(&self) -> Result<Key, HistoryError> {
        Ok(Key::parse(Some(&self.created_at), Some(&self.id))?)
    }

    pub fn id_bytes(&self) -> Result<[u8; 16], HistoryError> {
        self.key().map(Key::id_bytes)
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

#[cfg(test)]
mod tests {
    //! Precision and tuple-order regressions; transport round-trips are API tests.
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
