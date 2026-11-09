//! Owns exact backend pagination boundaries shared by list endpoints; caching belongs to the SDK.

use crate::{history::HistoryOrder, routes};

/// A backend cursor. `since` is sent verbatim; `last_id` breaks equal-timestamp ties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor<'a> {
    /// Verbatim backend timestamp, including fractional precision and offset.
    pub since: &'a str,
    /// Opaque backend identifier paired with `since` when the endpoint requires a tuple.
    pub last_id: Option<&'a str>,
    /// Ordering direction; omission leaves the endpoint's server-side default.
    pub order: Option<HistoryOrder>,
}

impl Cursor<'_> {
    pub(crate) fn parameters(&self) -> Vec<(&'static str, String)> {
        let mut parameters = Vec::new();
        if let Some(order) = self.order {
            parameters.push((routes::ORDER, order.as_str().to_owned()));
        }
        parameters.push((routes::SINCE, self.since.to_owned()));
        if let Some(last_id) = self.last_id {
            parameters.push((routes::LAST_ID, last_id.to_owned()));
        }
        parameters
    }
}

#[cfg(test)]
mod tests {
    //! Verifies exact cursor serialization without clocks or network access.

    use super::*;

    #[test]
    fn exact_tuple_and_order_are_preserved() {
        let cursor = Cursor {
            since: "2026-01-01T01:02:03.123456+03:00",
            last_id: Some("00000000-0000-0000-0000-000000000000"),
            order: Some(HistoryOrder::Asc),
        };
        assert_eq!(
            cursor.parameters(),
            vec![
                ("order", "asc".to_owned()),
                ("since", "2026-01-01T01:02:03.123456+03:00".to_owned()),
                ("last_id", "00000000-0000-0000-0000-000000000000".to_owned())
            ]
        );
    }

    #[test]
    fn timestamp_only_polling_omits_id_and_default_order() {
        let cursor = Cursor {
            since: "2026-01-01T00:00:00Z",
            last_id: None,
            order: None,
        };
        assert_eq!(
            cursor.parameters(),
            vec![("since", "2026-01-01T00:00:00Z".to_owned())]
        );
    }
}
