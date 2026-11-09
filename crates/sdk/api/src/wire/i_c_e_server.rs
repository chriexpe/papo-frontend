//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ICEServer`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ICEServer {
    /// Backend `urls` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#urls: Option<Vec<String>>,
    /// Backend `username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#username: Option<String>,
    /// Backend `credential` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#credential: Option<String>,
}
