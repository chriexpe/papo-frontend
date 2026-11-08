//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedMedia`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedMedia {
    /// Backend `url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#url: Option<String>,
    /// Backend `mime_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mime_type: Option<String>,
    /// Backend `width` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#width: Option<i64>,
    /// Backend `height` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#height: Option<i64>,
    /// Backend `size_bytes` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#size_bytes: Option<i64>,
}
