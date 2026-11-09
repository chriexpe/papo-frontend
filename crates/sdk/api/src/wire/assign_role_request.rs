//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `AssignRoleRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AssignRoleRequest {
    /// Backend `role_id` field.
    pub r#role_id: String,
}
