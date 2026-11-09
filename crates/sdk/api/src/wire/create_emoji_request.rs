//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateEmojiRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateEmojiRequest {
    /// Backend `name` field.
    pub r#name: String,
    /// Backend `image_blob` field.
    pub r#image_blob: String,
    /// Backend `format` field.
    pub r#format: String,
}
