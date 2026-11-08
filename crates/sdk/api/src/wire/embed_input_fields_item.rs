//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedInputFieldsItem`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedInputFieldsItem {
    /// Backend `name` field.
    pub r#name: String,
    /// Backend `value` field.
    pub r#value: String,
    /// Backend `inline` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#inline: Option<bool>,
}
