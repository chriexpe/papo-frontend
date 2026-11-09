//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ReplaceServerRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ReplaceServerRequest {
    /// Backend `name` field.
    pub r#name: String,
    /// Backend `icon_blob` field.
    pub r#icon_blob: String,
    /// Backend `icon_format` field.
    pub r#icon_format: String,
    /// Backend `password` field.
    pub r#password: String,
    /// Backend `public` field.
    pub r#public: bool,
}
