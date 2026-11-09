//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `SearchRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SearchRequest {
    /// Backend `text` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#text: Option<String>,
    /// Backend `author` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author: Option<String>,
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `mention` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mention: Option<String>,
    /// Backend `has` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has: Option<String>,
    /// Backend `order` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#order: Option<String>,
    /// Backend `date_start` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#date_start: Option<String>,
    /// Backend `date_end` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#date_end: Option<String>,
    /// Backend `contains_attachment` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#contains_attachment: Option<bool>,
}
