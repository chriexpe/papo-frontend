//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Role`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Role {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `color` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#color: Option<String>,
    /// Backend `permissions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#permissions: Option<super::RolePermissions>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
