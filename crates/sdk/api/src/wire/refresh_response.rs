//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `RefreshResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RefreshResponse {
    /// Backend `connection` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#connection: Option<super::Connection>,
}
