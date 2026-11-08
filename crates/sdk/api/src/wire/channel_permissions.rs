//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelPermissions`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelPermissions {
    /// Backend `read_channel` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#read_channel: Option<bool>,
    /// Backend `send_messages` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#send_messages: Option<bool>,
    /// Backend `delete_messages` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#delete_messages: Option<bool>,
    /// Backend `connect_voice` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#connect_voice: Option<bool>,
}
