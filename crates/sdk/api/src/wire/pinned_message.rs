//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `PinnedMessage`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PinnedMessage {
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `message_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#message_id: Option<String>,
    /// Backend `pinned_by` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#pinned_by: Option<String>,
    /// Backend `pinned_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#pinned_at: Option<String>,
}
