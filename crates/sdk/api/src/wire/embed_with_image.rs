//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedWithImage`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedWithImage {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `source_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#source_type: Option<String>,
    /// Backend `fetch_method` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#fetch_method: Option<String>,
    /// Backend `provider` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#provider: Option<String>,
    /// Backend `site_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#site_name: Option<String>,
    /// Backend `url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#url: Option<String>,
    /// Backend `title` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#title: Option<String>,
    /// Backend `description` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#description: Option<String>,
    /// Backend `color` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#color: Option<String>,
    /// Backend `author` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author: Option<super::EmbedAuthor>,
    /// Backend `thumbnail` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#thumbnail: Option<super::EmbedMedia>,
    /// Backend `image` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#image: Option<super::EmbedMedia>,
    /// Backend `video` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#video: Option<super::EmbedMedia>,
    /// Backend `footer` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#footer: Option<super::EmbedFooter>,
    /// Backend `fields` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#fields: Option<Vec<super::EmbedField>>,
    /// Backend `embed_url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#embed_url: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `fetched_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#fetched_at: Option<String>,
    /// Backend `image_data` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#image_data: Option<String>,
    /// Backend `author_image_data` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_image_data: Option<String>,
}
