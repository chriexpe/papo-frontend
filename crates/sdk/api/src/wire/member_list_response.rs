//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `MemberListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MemberListResponse {
    /// Backend `users` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#users: Option<Vec<super::UserSummary>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
