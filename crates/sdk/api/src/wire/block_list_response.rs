//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `BlockListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct BlockListResponse {
    /// Backend `users` field.
    pub r#users: Vec<super::UserSummary>,
}
