//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UserConfig`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserConfig {
    /// Backend `theme` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#theme: Option<String>,
    /// Backend `notifications` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#notifications: Option<super::Notifications>,
    /// Backend `display` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#display: Option<super::Display>,
}
