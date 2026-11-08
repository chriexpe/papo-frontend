//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedInputFooter`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedInputFooter {
    /// Backend `text` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#text: Option<String>,
}
