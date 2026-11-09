//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MessageReactionGroup`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageReactionGroup {
    /// Backend `emoji_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#emoji_id: Option<String>,
    /// Backend `unicode` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#unicode: Option<String>,
    /// Backend `count` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#count: Option<i64>,
    /// Backend `users` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#users: Option<Vec<super::MessageReactionUser>>,
}
