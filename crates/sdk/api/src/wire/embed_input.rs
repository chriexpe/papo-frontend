//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `EmbedInput`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EmbedInput {
    /// Backend `title` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#title: Option<String>,
    /// Backend `description` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#description: Option<String>,
    /// Backend `url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#url: Option<String>,
    /// Backend `color` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#color: Option<String>,
    /// Backend `site_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#site_name: Option<String>,
    /// Backend `author` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author: Option<super::EmbedInputAuthor>,
    /// Backend `footer` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#footer: Option<super::EmbedInputFooter>,
    /// Backend `thumbnail` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#thumbnail: Option<super::EmbedMediaInput>,
    /// Backend `image` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#image: Option<super::EmbedMediaInput>,
    /// Backend `video` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#video: Option<super::EmbedMediaInput>,
    /// Backend `fields` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#fields: Option<Vec<super::EmbedInputFieldsItem>>,
}
