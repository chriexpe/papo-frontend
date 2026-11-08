//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedMediaInput`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedMediaInput {
    /// Backend `url` field.
    pub r#url: String,
    /// Backend `mime_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mime_type: Option<String>,
}
