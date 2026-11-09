//! Owns social and message-interaction REST contracts; policy and state belong to the SDK.

mod conversations;
mod interactions;
mod notifications;

use serde::Serialize;

/// Exact pagination boundary returned by the backend; timestamp text is not reformatted.
#[derive(Clone, Debug, Serialize)]
pub struct Cursor<'a> {
    /// Verbatim backend timestamp.
    pub since: &'a str,
    /// Opaque backend identifier paired with the timestamp.
    pub last_id: &'a str,
}

/// Pagination direction for notifications and reaction users.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    /// Most recent first.
    #[default]
    Desc,
    /// Oldest first.
    Asc,
}

/// Cursor and direction for notifications and reaction users.
#[derive(Default)]
pub struct Page<'a> {
    /// A complete exact tuple boundary, omitted for the initial page.
    pub cursor: Option<Cursor<'a>>,
    /// Timestamp-only polling boundary; ignored when `cursor` is supplied.
    pub since: Option<&'a str>,
    /// The server's ordering direction.
    pub order: Order,
}

impl Page<'_> {
    fn parameters(&self) -> Vec<(&'static str, &str)> {
        let mut values = vec![(
            "order",
            match self.order {
                Order::Asc => "asc",
                Order::Desc => "desc",
            },
        )];
        if let Some(cursor) = &self.cursor {
            values.push(("since", cursor.since));
            values.push(("last_id", cursor.last_id));
        } else if let Some(since) = self.since {
            values.push(("since", since));
        }
        values
    }
}

#[cfg(test)]
mod tests {
    //! Owns deterministic cursor serialization tests.
    use super::*;

    #[test]
    fn cursor_preserves_timestamp_and_id() {
        let page = Page {
            cursor: Some(Cursor {
                since: "2026-01-01T01:02:03.123456+03:00",
                last_id: "00000000-0000-0000-0000-000000000000",
            }),
            since: None,
            order: Order::Asc,
        };
        assert_eq!(
            page.parameters(),
            vec![
                ("order", "asc"),
                ("since", "2026-01-01T01:02:03.123456+03:00"),
                ("last_id", "00000000-0000-0000-0000-000000000000")
            ]
        );
    }

    #[test]
    fn timestamp_only_polling_omits_last_id() {
        let page = Page {
            since: Some("2026-01-01T00:00:00Z"),
            ..Page::default()
        };
        assert_eq!(
            page.parameters(),
            vec![("order", "desc"), ("since", "2026-01-01T00:00:00Z")]
        );
    }

    #[test]
    fn initial_page_has_no_cursor_fields() {
        assert_eq!(Page::default().parameters(), vec![("order", "desc")]);
    }
}
