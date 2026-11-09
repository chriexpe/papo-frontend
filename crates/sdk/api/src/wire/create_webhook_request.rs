//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `CreateWebhookRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct CreateWebhookRequest {
    /// Backend `channel_id` field.
    pub r#channel_id: String,
    /// Backend `name` field.
    pub r#name: String,
}
