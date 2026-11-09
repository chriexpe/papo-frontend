//! Owns audit-log filtering and pagination; authorization remains a backend decision.

use crate::{Cursor, Error, RestClient, routes};

/// Audit filters; the cursor carries `since`, `last_id` and order separately.
#[derive(Clone, Default)]
pub struct AuditLogQuery<'a> {
    /// Exact backend action.
    pub action: Option<&'a str>,
    /// Actor UUID.
    pub actor_id: Option<&'a str>,
    /// Backend entity type.
    pub entity_type: Option<&'a str>,
    /// Inclusive RFC 3339 upper timestamp bound.
    pub until: Option<&'a str>,
}

impl AuditLogQuery<'_> {
    fn parameters(&self) -> Vec<(&'static str, String)> {
        [
            (routes::ACTION, self.action),
            (routes::ACTOR_ID, self.actor_id),
            (routes::ENTITY_TYPE, self.entity_type),
            (routes::UNTIL, self.until),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value.to_owned())))
        .collect()
    }
}

impl RestClient {
    /// Fetches an audit page without applying local filters or changing the cursor.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn audit_logs(
        &self,
        query: &AuditLogQuery<'_>,
        cursor: Option<&Cursor<'_>>,
    ) -> Result<crate::wire::AuditLogList, Error> {
        let mut parameters = query.parameters();
        if let Some(cursor) = cursor {
            parameters.extend(cursor.parameters());
        }
        self.json(
            self.route_request(&routes::GET_ADMIN_AUDIT_LOGS, &[])?
                .query(&parameters),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies filter and cursor query assembly without network access.

    use super::*;
    use crate::HistoryOrder;

    #[test]
    fn filters_and_cursor_share_generated_query_names() {
        let filters = AuditLogQuery {
            action: Some("delete"),
            until: Some("2030-01-01T00:00:00Z"),
            ..AuditLogQuery::default()
        };
        let cursor = Cursor {
            since: "2026-01-01T01:02:03.123456+03:00",
            last_id: Some("cursor"),
            order: Some(HistoryOrder::Desc),
        };
        let mut parameters = filters.parameters();
        parameters.extend(cursor.parameters());
        assert_eq!(
            parameters,
            vec![
                ("action", "delete".to_owned()),
                ("until", "2030-01-01T00:00:00Z".to_owned()),
                ("order", "desc".to_owned()),
                ("since", "2026-01-01T01:02:03.123456+03:00".to_owned()),
                ("last_id", "cursor".to_owned()),
            ]
        );
    }
}
