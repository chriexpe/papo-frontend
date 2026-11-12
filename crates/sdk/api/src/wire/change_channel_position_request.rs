//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChangeChannelPositionRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChangeChannelPositionRequest {
    /// Backend `old_position` field.
    pub r#old_position: i64,
    /// Backend `new_position` field.
    pub r#new_position: i64,
    /// Backend `parent_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#parent_id: Option<String>,
}
