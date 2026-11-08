//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `DropConnectionRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DropConnectionRequest {
    /// Backend `connection_id` field.
    pub r#connection_id: String,
}
