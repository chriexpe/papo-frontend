//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MessageListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageListResponse {
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `messages` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#messages: Option<Vec<super::Message>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
