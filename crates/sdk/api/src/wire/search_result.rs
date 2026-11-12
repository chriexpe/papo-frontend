//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `SearchResult`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SearchResult {
    /// Backend `type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `content` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#content: Option<String>,
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `channel_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_name: Option<String>,
    /// Backend `author_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_id: Option<String>,
    /// Backend `author_username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_username: Option<String>,
    /// Backend `webhook_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#webhook_id: Option<String>,
    /// Backend `external_author` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#external_author: Option<super::ExternalAuthor>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `score` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#score: Option<f64>,
}
