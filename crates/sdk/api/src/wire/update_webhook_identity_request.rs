//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateWebhookIdentityRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateWebhookIdentityRequest {
    /// Backend `display_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#display_name: Option<String>,
    /// Backend `avatar` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#avatar: Option<String>,
    /// Backend `avatar_format` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#avatar_format: Option<String>,
}
