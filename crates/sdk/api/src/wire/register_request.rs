//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `RegisterRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RegisterRequest {
    /// Backend `username` field.
    pub r#username: String,
    /// Backend `password` field.
    pub r#password: String,
}
