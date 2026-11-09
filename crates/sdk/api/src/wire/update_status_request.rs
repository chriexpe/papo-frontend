//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateStatusRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateStatusRequest {
    /// Backend `status` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#status: Option<String>,
}
