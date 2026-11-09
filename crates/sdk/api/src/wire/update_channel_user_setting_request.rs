//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateChannelUserSettingRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateChannelUserSettingRequest {
    /// Backend `notification_settings` field.
    pub r#notification_settings: super::NotificationSettings,
}
