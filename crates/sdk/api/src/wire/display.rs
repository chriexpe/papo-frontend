//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Display`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Display {
    /// Backend `fontSize` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fontSize")]
    pub r#font_size: Option<String>,
    /// Backend `messageDensity` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "messageDensity")]
    pub r#message_density: Option<String>,
    /// Backend `showTimestamps` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "showTimestamps")]
    pub r#show_timestamps: Option<bool>,
    /// Backend `showAvatars` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "showAvatars")]
    pub r#show_avatars: Option<bool>,
}
