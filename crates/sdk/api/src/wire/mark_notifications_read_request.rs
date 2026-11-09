//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MarkNotificationsReadRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MarkNotificationsReadRequest {
    /// Backend `notification_ids` field.
    pub r#notification_ids: Vec<String>,
}
