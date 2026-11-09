//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateAvatarRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateAvatarRequest {
    /// Backend `avatar` field.
    pub r#avatar: String,
    /// Backend `avatar_format` field.
    pub r#avatar_format: String,
}
