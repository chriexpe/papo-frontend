//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmojiListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmojiListResponse {
    /// Backend `emojis` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#emojis: Option<Vec<super::Emoji>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
