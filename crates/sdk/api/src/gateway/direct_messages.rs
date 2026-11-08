//! Generated gateway payloads; leaves transport and policy outside.
/// Backend `dm_update` payload; contains no client state.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DMUpdateEvent {
    /// Backend `dm` field.
    pub dm: crate::wire::DirectConversation,
}
