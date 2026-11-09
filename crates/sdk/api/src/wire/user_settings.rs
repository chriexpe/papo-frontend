//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UserSettings`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserSettings {
    /// Backend `user_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user_id: Option<String>,
    /// Backend `version` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#version: Option<i64>,
    /// Backend `config` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#config: Option<super::UserConfig>,
    /// Backend `updated_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#updated_at: Option<String>,
}
