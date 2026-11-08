//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `LoginServerRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct LoginServerRequest {
    /// Backend `server_password` field.
    pub r#server_password: String,
}
