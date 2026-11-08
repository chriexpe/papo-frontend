//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `typing` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct TypingEvent {
    /// Backend `channel_id` field.
    pub channel_id: String,
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `is_typing` field.
    pub is_typing: bool,
}
/// Backend `avatar_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AvatarUpdateEvent {
    /// Backend `user_id` field.
    pub user_id: String,
}
/// Backend `presence_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PresenceUpdateEvent {
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `status` field.
    pub status: String,
    /// Backend `status_message` field.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Backend `typing` field.
    #[serde(default)]
    pub typing: Option<String>,
    /// Backend `nickname` field.
    #[serde(default)]
    pub nickname: Option<String>,
}
/// Backend `presence_sync` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PresenceSyncEvent {
    /// Backend `members` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub members: Vec<PresenceMember>,
}
/// Backend `_PresenceMember` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct PresenceMember {
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `status` field.
    pub status: String,
    /// Backend `status_message` field.
    #[serde(default)]
    pub status_message: Option<String>,
    /// Backend `user_voice` field.
    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]
    pub user_voice: Vec<String>,
}
