//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `RoleListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RoleListResponse {
    /// Backend `roles` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#roles: Option<Vec<super::Role>>,
}
