//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MessageReaction`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageReaction {
    /// Backend `message_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#message_id: Option<String>,
    /// Backend `user_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user_id: Option<String>,
    /// Backend `emoji_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#emoji_id: Option<String>,
    /// Backend `unicode` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#unicode: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
