//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MarkNotificationsReadResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MarkNotificationsReadResponse {
    /// Backend `updated` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#updated: Option<i64>,
}
