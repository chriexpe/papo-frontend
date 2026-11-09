//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Emoji`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Emoji {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `format` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#format: Option<String>,
    /// Backend `image_blob` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#image_blob: Option<String>,
    /// Backend `created_by` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_by: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
