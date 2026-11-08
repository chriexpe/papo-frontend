//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `WhoamiResponseSettings`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct WhoamiResponseSettings {
    /// Backend `version` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#version: Option<i64>,
    /// Backend `config` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#config: Option<super::UserConfig>,
}
