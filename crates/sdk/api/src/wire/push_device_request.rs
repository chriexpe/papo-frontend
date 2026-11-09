//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `PushDeviceRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PushDeviceRequest {
    /// Backend `token` field.
    pub r#token: String,
    /// Backend `platform` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#platform: Option<String>,
    /// Backend `device_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#device_name: Option<String>,
    /// Backend `provider` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#provider: Option<String>,
}
