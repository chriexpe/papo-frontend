//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `PasswordResetConsumeRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PasswordResetConsumeRequest {
    /// Backend `token` field.
    pub r#token: String,
    /// Backend `password` field.
    pub r#password: String,
}
