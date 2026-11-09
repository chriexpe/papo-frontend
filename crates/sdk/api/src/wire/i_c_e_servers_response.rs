//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ICEServersResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ICEServersResponse {
    /// Backend `ice_servers` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ice_servers: Option<Vec<super::ICEServer>>,
}
