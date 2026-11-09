//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelPermissionsResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelPermissionsResponse {
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `permissions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#permissions: Option<Vec<super::ChannelPermissionEntry>>,
}
