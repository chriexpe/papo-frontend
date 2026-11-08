//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `WhoamiResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct WhoamiResponse {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#username: Option<String>,
    /// Backend `nickname` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#nickname: Option<String>,
    /// Backend `avatar_blob` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#avatar_blob: Option<String>,
    /// Backend `avatar_format` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#avatar_format: Option<String>,
    /// Backend `status` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#status: Option<String>,
    /// Backend `status_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#status_message: Option<String>,
    /// Backend `typing` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#typing: Option<String>,
    /// Backend `status_updated_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#status_updated_at: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `roles` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#roles: Option<Vec<super::RoleSummary>>,
    /// Backend `settings` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#settings: Option<super::WhoamiResponseSettings>,
    /// Backend `connection_violation` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#connection_violation: Option<bool>,
}
