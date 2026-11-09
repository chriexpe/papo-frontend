//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Webhook`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Webhook {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `display_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#display_name: Option<String>,
    /// Backend `created_by` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_by: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `revoked_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#revoked_at: Option<String>,
}
