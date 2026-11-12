//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedMediaInput`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedMediaInput {
    /// Backend `url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#url: Option<String>,
    /// Backend `attachment_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#attachment_name: Option<String>,
    /// Backend `mime_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mime_type: Option<String>,
}
