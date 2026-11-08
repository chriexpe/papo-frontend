//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `user_join` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UserJoinEvent {
    /// Backend `user_id` field.
    pub user_id: String,
}
/// Backend `role_add` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RoleAddEvent {
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `role_id` field.
    pub role_id: String,
}
/// Backend `role_remove` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct RoleRemoveEvent {
    /// Backend `user_id` field.
    pub user_id: String,
    /// Backend `role_id` field.
    pub role_id: String,
}
