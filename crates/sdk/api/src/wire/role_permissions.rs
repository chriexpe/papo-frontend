//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `RolePermissions`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RolePermissions {
    /// Backend `manage_server` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#manage_server: Option<bool>,
    /// Backend `manage_channels` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#manage_channels: Option<bool>,
    /// Backend `manage_roles` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#manage_roles: Option<bool>,
    /// Backend `manage_webhooks` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#manage_webhooks: Option<bool>,
    /// Backend `ban_members` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ban_members: Option<bool>,
    /// Backend `pin_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#pin_message: Option<bool>,
    /// Backend `everyone_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#everyone_message: Option<bool>,
    /// Backend `send_attachment` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#send_attachment: Option<bool>,
}
