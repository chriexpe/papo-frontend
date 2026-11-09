//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UserRole`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserRole {
    /// Backend `user_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user_id: Option<String>,
    /// Backend `role_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#role_id: Option<String>,
    /// Backend `assigned_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#assigned_at: Option<String>,
}
