//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedField`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedField {
    /// Backend `position` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#position: Option<i64>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `value` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#value: Option<String>,
    /// Backend `inline` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#inline: Option<bool>,
}
