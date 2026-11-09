//! Owns audit-log pagination; filtering and authorization remain backend decisions.

use crate::{Error, RestClient, wire::AuditLogList};
use reqwest::Method;

/// Audit filters and the raw backend cursor. Timestamps are sent unchanged.
#[derive(Clone, Default, serde::Serialize)]
pub struct AuditLogQuery<'a> {
    /// Exact backend action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'a str>,
    /// Actor UUID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<&'a str>,
    /// Backend entity type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<&'a str>,
    /// Inclusive RFC 3339 lower timestamp bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<&'a str>,
    /// Inclusive RFC 3339 upper timestamp bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<&'a str>,
    /// Last raw log UUID, including when client-side filtering hides it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<&'a str>,
    /// Backend ordering, `asc` or `desc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<&'a str>,
}

impl RestClient {
    /// Fetches an audit page without changing its cursor or applying local filters.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn audit_logs(&self, query: &AuditLogQuery<'_>) -> Result<AuditLogList, Error> {
        self.json(self.request(Method::GET, "admin/audit-logs")?.query(query))
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Tests audit cursor serialization without exercising transport.

    use super::AuditLogQuery;

    #[test]
    fn audit_cursor_is_preserved_as_sent() {
        let query = AuditLogQuery {
            since: Some("2030-01-01T00:00:00.123456Z"),
            last_id: Some("cursor"),
            ..Default::default()
        };
        let value = serde_json::to_value(query).unwrap();
        assert_eq!(value["since"], "2030-01-01T00:00:00.123456Z");
        assert_eq!(value["last_id"], "cursor");
        assert!(value.get("until").is_none());
    }
}
