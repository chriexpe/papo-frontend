//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `LoginResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct LoginResponse {
    /// Backend `user` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user: Option<super::LoginResponseUser>,
    /// Backend `connection_violation` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#connection_violation: Option<bool>,
}
