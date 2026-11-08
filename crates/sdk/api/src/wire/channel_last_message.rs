//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelLastMessage`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelLastMessage {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `content` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#content: Option<String>,
    /// Backend `author_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_id: Option<String>,
    /// Backend `author_username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_username: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
