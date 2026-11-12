//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateChannelRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateChannelRequest {
    /// Backend `name` field.
    pub r#name: String,
    /// Backend `type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Backend `topic` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#topic: Option<String>,
    /// Backend `parent_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#parent_id: Option<String>,
    /// Backend `access_roles` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#access_roles: Option<Vec<String>>,
}
