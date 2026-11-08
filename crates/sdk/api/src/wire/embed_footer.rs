//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedFooter`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedFooter {
    /// Backend `text` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#text: Option<String>,
    /// Backend `icon` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#icon: Option<super::EmbedMedia>,
}
