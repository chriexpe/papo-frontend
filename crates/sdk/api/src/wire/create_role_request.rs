//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateRoleRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateRoleRequest {
    /// Backend `name` field.
    pub r#name: String,
    /// Backend `color` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#color: Option<String>,
    /// Backend `permissions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#permissions: Option<super::RolePermissions>,
    /// Backend `users` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#users: Option<Vec<String>>,
}
