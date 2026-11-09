//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UserSummaryBatchRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserSummaryBatchRequest {
    /// Backend `ids` field.
    pub r#ids: Vec<String>,
}
