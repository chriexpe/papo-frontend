//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ConnectedDevicesResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ConnectedDevicesResponse {
    /// Backend `connections` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#connections: Option<Vec<super::Connection>>,
}
