//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateChannelPermissionsRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateChannelPermissionsRequest {
    /// Backend `permissions` field.
    pub r#permissions: super::ChannelPermissions,
}
