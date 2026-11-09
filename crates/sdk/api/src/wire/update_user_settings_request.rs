//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateUserSettingsRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateUserSettingsRequest {
    /// Backend `config` field.
    pub r#config: super::UserConfig,
}
