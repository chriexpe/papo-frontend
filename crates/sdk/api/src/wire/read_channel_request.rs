//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ReadChannelRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ReadChannelRequest {
    /// Backend `message_id` field.
    pub r#message_id: String,
}
