//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `OpenDirectMessageRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct OpenDirectMessageRequest {
    /// Backend `user_id` field.
    pub r#user_id: String,
}
