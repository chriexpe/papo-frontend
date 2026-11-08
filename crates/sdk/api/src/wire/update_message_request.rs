//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateMessageRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateMessageRequest {
    /// Backend `content` field.
    pub r#content: String,
    /// Backend `embeds` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#embeds: Option<Vec<super::EmbedInput>>,
}
