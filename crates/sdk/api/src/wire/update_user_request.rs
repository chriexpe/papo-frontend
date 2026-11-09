//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateUserRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateUserRequest {
    /// Backend `nickname` field.
    pub r#nickname: String,
    /// Backend `status` field.
    pub r#status: String,
    /// Backend `description` field.
    pub r#description: String,
    /// Backend `typing` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#typing: Option<String>,
}
