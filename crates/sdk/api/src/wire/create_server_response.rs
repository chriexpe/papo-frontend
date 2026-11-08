//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateServerResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateServerResponse {
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
    /// Backend `owner_username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#owner_username: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
