//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedAuthor`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedAuthor {
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#url: Option<String>,
    /// Backend `media` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#media: Option<super::EmbedMedia>,
}
