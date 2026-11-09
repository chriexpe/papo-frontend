//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MessageReactionList`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageReactionList {
    /// Backend `message_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#message_id: Option<String>,
    /// Backend `reactions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#reactions: Option<Vec<super::MessageReactionGroup>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
