//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `message` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageEvent {
    /// Backend `id` field.
    pub id: String,
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `author_id` field.
    pub author_id: String,
    /// Backend `webhook_id` field.
    #[serde(default)]
    pub webhook_id: Option<String>,
    /// Backend `external_author` field.
    #[serde(default)]
    pub external_author: Option<crate::wire::ExternalAuthor>,
    /// Backend `content` field.
    pub content: String,
    /// Backend `created_at` field.
    pub created_at: String,
    /// Backend `reply_to` field.
    pub reply_to: Option<String>,
    /// Backend `attachments` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub attachments: Vec<crate::wire::Attachment>,
}
/// Backend `message_edit` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageEditEvent {
    /// Backend `id` field.
    pub id: String,
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `content` field.
    pub content: String,
    /// Backend `edited_at` field.
    pub edited_at: String,
}
/// Backend `message_delete` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageDeleteEvent {
    /// Backend `id` field.
    pub id: String,
    /// Backend `channel_id` field.
    pub channel_id: String,
}
/// Backend `attachment_moderation_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AttachmentModerationUpdateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `message_id` field.
    pub message_id: String,
    /// Backend `attachment_id` field.
    pub attachment_id: String,
    /// Backend `status` field.
    pub status: String,
}
/// Backend `message_embeds_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessageEmbedsUpdateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `message_id` field.
    pub message_id: String,
    /// Backend `embeds` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub embeds: Vec<crate::wire::Embed>,
}
/// Backend `react_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ReactUpdateEvent {
    /// Backend `message_id` field.
    pub message_id: String,
    /// Backend `emoji_id` field.
    pub emoji_id: Option<String>,
    /// Backend `unicode` field.
    pub unicode: Option<String>,
    /// Backend `count` field.
    pub count: i64,
}
/// Backend `message_pin` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct MessagePinEvent {
    /// Backend `message_id` field.
    pub message_id: String,
    /// Backend `is_pinned` field.
    pub is_pinned: bool,
}
