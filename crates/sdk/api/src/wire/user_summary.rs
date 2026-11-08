//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UserSummary`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserSummary {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#username: Option<String>,
    /// Backend `nickname` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#nickname: Option<String>,
    /// Backend `banned` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#banned: Option<bool>,
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
}
