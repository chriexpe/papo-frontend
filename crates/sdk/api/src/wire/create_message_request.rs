//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateMessageRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateMessageRequest {
    /// Backend `channel_id` field.
    pub r#channel_id: String,
    /// Backend `content` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#content: Option<String>,
    /// Backend `reply_to` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#reply_to: Option<String>,
    /// Backend `attachments` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#attachments: Option<Vec<Vec<u8>>>,
    /// Backend `embeds` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#embeds: Option<String>,
}
