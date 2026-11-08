//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `new_notification` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct NewNotificationEvent {
    /// Backend `id` field.
    pub id: String,
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `message_id` field.
    pub message_id: String,
    /// Backend `message_content` field.
    pub message_content: String,
}
