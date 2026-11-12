//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `channel_create` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelCreateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `name` field.
    pub name: String,
    /// Backend `channel_type` field.
    pub channel_type: String,
    /// Backend `position` field.
    pub position: i64,
    /// Backend `topic` field.
    pub topic: Option<String>,
    /// Backend `parent_id` field.
    pub parent_id: Option<String>,
}
/// Backend `channel_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelUpdateEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `name` field.
    pub name: String,
    /// Backend `position` field.
    pub position: i64,
    /// Backend `topic` field.
    pub topic: Option<String>,
    /// Backend `parent_id` field.
    pub parent_id: Option<String>,
}
/// Backend `channel_delete` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelDeleteEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
}
