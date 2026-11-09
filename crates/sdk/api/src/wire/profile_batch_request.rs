//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ProfileBatchRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ProfileBatchRequest {
    /// Backend `ids` field.
    pub r#ids: Vec<String>,
}
