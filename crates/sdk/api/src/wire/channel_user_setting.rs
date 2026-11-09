//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelUserSetting`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelUserSetting {
    /// Backend `user_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user_id: Option<String>,
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `notification_settings` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#notification_settings: Option<super::NotificationSettings>,
    /// Backend `updated_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#updated_at: Option<String>,
}
