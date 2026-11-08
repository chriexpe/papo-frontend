//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelPermissionEntry`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelPermissionEntry {
    /// Backend `role_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#role_id: Option<String>,
    /// Backend `role_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#role_name: Option<String>,
    /// Backend `permissions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#permissions: Option<super::ChannelPermissions>,
}
