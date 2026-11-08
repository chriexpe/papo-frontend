//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Server`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Server {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `icon_blob` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#icon_blob: Option<String>,
    /// Backend `icon_format` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#icon_format: Option<String>,
    /// Backend `owner_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#owner_id: Option<String>,
    /// Backend `public` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#public: Option<bool>,
    /// Backend `owner_username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#owner_username: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `channel_count` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_count: Option<i64>,
    /// Backend `member_count` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#member_count: Option<i64>,
    /// Backend `role_count` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#role_count: Option<i64>,
}
