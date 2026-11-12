//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Channel`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Channel {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Backend `position` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#position: Option<i64>,
    /// Backend `topic` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#topic: Option<String>,
    /// Backend `parent_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#parent_id: Option<String>,
    /// Backend `permissions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#permissions: Option<Vec<super::ChannelPermissionEntry>>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `last_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_message: Option<super::ChannelLastMessage>,
    /// Backend `last_read_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_read_message: Option<String>,
    /// Backend `last_read_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_read_at: Option<String>,
    /// Backend `notification_settings` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#notification_settings: Option<super::NotificationSettings>,
}
