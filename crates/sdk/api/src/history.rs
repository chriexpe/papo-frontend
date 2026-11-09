//! Owns exact history cursor transport; ordering and cache continuity belong to the SDK.

use crate::routes;
use serde::Serialize;

/// Exact backend cursor. Never parse/reformat `created_at` before sending it back.
#[derive(Clone, PartialEq, Eq)]
pub struct HistoryCursor {
    /// Verbatim server timestamp, including fractional precision and offset.
    pub created_at: String,
    /// Opaque backend message UUID string, used to break equal-timestamp ties.
    pub id: String,
}

impl crate::wire::Message {
    /// Returns a cursor only when the backend supplied both ordering fields.
    #[must_use]
    pub fn cursor(&self) -> Option<HistoryCursor> {
        Some(HistoryCursor {
            created_at: self.created_at.clone()?,
            id: self.id.clone()?,
        })
    }
}

/// Direction of the backend's strict tuple comparison.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOrder {
    /// Older messages, newest first.
    #[default]
    Desc,
    /// Newer messages, oldest first.
    Asc,
}

impl HistoryOrder {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// One history request; the server validates the page size (1 through 100).
#[derive(Default)]
pub struct HistoryQuery<'a> {
    /// Tuple boundary, excluded from the response in either direction.
    pub cursor: Option<&'a HistoryCursor>,
    /// Ordering direction.
    pub order: HistoryOrder,
    /// Page size in messages. Omission uses the server default.
    pub limit: Option<u8>,
}

impl HistoryQuery<'_> {
    pub(crate) fn parameters(&self) -> Vec<(&'static str, String)> {
        let mut parameters = vec![(routes::ORDER, self.order.as_str().to_owned())];
        if let Some(cursor) = self.cursor {
            parameters.push((routes::SINCE, cursor.created_at.clone()));
            parameters.push((routes::LAST_ID, cursor.id.clone()));
        }
        if let Some(limit) = self.limit {
            parameters.push((routes::LIMIT, limit.to_string()));
        }
        parameters
    }
}

#[cfg(test)]
mod tests {
    //! Owns exact cursor serialization tests, without clocks or network.
    use super::*;
    #[test]
    fn fractional_timestamp_and_offset_round_trip_verbatim() {
        let cursor = HistoryCursor {
            created_at: "2026-01-01T01:02:03.123456+03:00".into(),
            id: uuid::Uuid::nil().to_string(),
        };
        for order in [HistoryOrder::Asc, HistoryOrder::Desc] {
            let query = HistoryQuery {
                cursor: Some(&cursor),
                order,
                limit: Some(2),
            };
            assert_eq!(query.parameters()[1].1, cursor.created_at);
            assert_eq!(query.parameters()[2].1, cursor.id);
        }
    }
}
