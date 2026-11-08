//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `DirectConversation`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DirectConversation {
    /// Backend `id` field.
    pub r#id: String,
    /// Backend `user` field.
    pub r#user: super::UserSummary,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    pub r#created_at: String,
    /// Backend `last_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_message: Option<super::ChannelLastMessage>,
    /// Backend `last_read_message` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_read_message: Option<String>,
    /// Backend `last_read_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#last_read_at: Option<String>,
    /// Backend `unread_count` field.
    pub r#unread_count: i64,
}
