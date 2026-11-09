//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `PasswordResetLinkResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PasswordResetLinkResponse {
    /// Backend `reset_url` field.
    pub r#reset_url: String,
    /// Backend `expires_at` field. Timestamp strings are preserved verbatim.
    pub r#expires_at: String,
}
