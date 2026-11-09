//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `PatchServerRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PatchServerRequest {
    /// Backend `name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#name: Option<String>,
    /// Backend `icon_blob` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#icon_blob: Option<String>,
    /// Backend `icon_format` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#icon_format: Option<String>,
    /// Backend `password` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#password: Option<String>,
    /// Backend `public` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#public: Option<bool>,
}
