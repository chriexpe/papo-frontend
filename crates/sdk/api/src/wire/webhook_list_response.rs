//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `WebhookListResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct WebhookListResponse {
    /// Backend `webhooks` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#webhooks: Option<Vec<super::Webhook>>,
}
