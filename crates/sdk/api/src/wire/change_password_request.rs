//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChangePasswordRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChangePasswordRequest {
    /// Backend `password` field.
    pub r#password: String,
}
