//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `heartbeat_ack` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct HeartbeatAckEvent {}
/// Backend `error` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ErrorEvent {
    /// Backend `message` field.
    pub message: String,
    /// Backend `code` field.
    #[serde(default)]
    pub code: Option<String>,
}
