//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Notifications`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Notifications {
    /// Backend `enabled` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#enabled: Option<bool>,
    /// Backend `messagePreview` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "messagePreview")]
    pub r#message_preview: Option<bool>,
    /// Backend `sound` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#sound: Option<bool>,
    /// Backend `mentions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mentions: Option<bool>,
}
