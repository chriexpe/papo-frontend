//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `DropConnectionResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DropConnectionResponse {
    /// Backend `dropped` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#dropped: Option<i64>,
}
