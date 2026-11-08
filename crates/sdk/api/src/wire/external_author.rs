//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ExternalAuthor`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ExternalAuthor {
    /// Backend `source` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#source: Option<String>,
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `avatar_url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#avatar_url: Option<String>,
    /// Backend `profile_url` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#profile_url: Option<String>,
}
