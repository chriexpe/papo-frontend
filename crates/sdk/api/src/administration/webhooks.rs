//! Owns webhook management and bearer-scoped message calls; token storage belongs to callers.

use crate::{
    Error, RestClient, Uuid, routes,
    wire::{
        CreateWebhookRequest, Message, UpdateMessageRequest, UpdateWebhookIdentityRequest, Webhook,
        WebhookListResponse,
    },
};
/// One-time creation response. The token is not returned by listing or updating.
#[derive(Clone, serde::Deserialize)]
pub struct CreatedWebhook {
    /// The created webhook fields.
    #[serde(flatten)]
    pub webhook: Webhook,
    /// The one-time scoped credential; never log or persist it as ordinary data.
    pub token: String,
}

/// One attachment for a webhook multipart message.
#[derive(Clone)]
pub struct WebhookAttachment {
    /// Filename sent to the backend.
    pub filename: String,
    /// File bytes.
    pub bytes: Vec<u8>,
}

/// Multipart fields for a webhook message. JSON fields contain serialized backend objects.
#[derive(Clone, Default)]
pub struct WebhookMessageRequest {
    /// Optional message text.
    pub content: Option<String>,
    /// Optional idempotency key scoped by the webhook.
    pub external_message_id: Option<String>,
    /// Serialized external author object.
    pub external_author: Option<String>,
    /// Optional reply target UUID.
    pub reply_to: Option<String>,
    /// Serialized array of embed inputs.
    pub embeds: Option<String>,
    /// Files with their original names.
    pub attachments: Vec<WebhookAttachment>,
}

impl RestClient {
    /// Creates a webhook and returns its credential once.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn create_webhook(
        &self,
        request: &CreateWebhookRequest,
    ) -> Result<CreatedWebhook, Error> {
        self.post(&routes::POST_WEBHOOKS, &[], request).await
    }

    /// Lists active webhooks.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn webhooks(&self) -> Result<WebhookListResponse, Error> {
        self.json(self.route_request(&routes::GET_WEBHOOKS, &[])?)
            .await
    }

    /// Updates the persistent identity of a webhook.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_webhook(
        &self,
        webhook: Uuid,
        request: &UpdateWebhookIdentityRequest,
    ) -> Result<Webhook, Error> {
        self.json(
            self.route_request(&routes::PATCH_WEBHOOKS_WEBHOOK_ID, &[webhook.to_string()])?
                .json(request),
        )
        .await
    }

    /// Revokes a webhook.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_webhook(&self, webhook: Uuid) -> Result<(), Error> {
        self.response(
            self.route_request(&routes::DELETE_WEBHOOKS_WEBHOOK_ID, &[webhook.to_string()])?,
        )
        .await
        .map(|_| ())
    }

    /// Sends a webhook message with only its scoped bearer credential. No retry is performed.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn execute_webhook(
        &self,
        webhook: Uuid,
        token: &str,
        request: &WebhookMessageRequest,
    ) -> Result<Message, Error> {
        let mut form = reqwest::multipart::Form::new();
        for (name, value) in [
            ("content", &request.content),
            ("external_message_id", &request.external_message_id),
            ("external_author", &request.external_author),
            ("reply_to", &request.reply_to),
            ("embeds", &request.embeds),
        ] {
            if let Some(value) = value {
                form = form.text(name, value.clone());
            }
        }
        for attachment in &request.attachments {
            form = form.part(
                "attachments",
                reqwest::multipart::Part::bytes(attachment.bytes.clone())
                    .file_name(attachment.filename.clone()),
            );
        }
        self.anonymous_json(
            self.anonymous_route_request(
                &routes::POST_WEBHOOKS_WEBHOOK_ID_MESSAGES,
                &[webhook.to_string()],
            )?
            .bearer_auth(token)
            .multipart(form),
        )
        .await
    }

    /// Edits a webhook-owned message with the scoped bearer credential.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn edit_webhook_message(
        &self,
        webhook: Uuid,
        message: Uuid,
        token: &str,
        request: &UpdateMessageRequest,
    ) -> Result<Message, Error> {
        self.anonymous_json(
            self.anonymous_route_request(
                &routes::PUT_WEBHOOKS_WEBHOOK_ID_MESSAGES_MESSAGE_ID,
                &[webhook.to_string(), message.to_string()],
            )?
            .bearer_auth(token)
            .json(request),
        )
        .await
    }

    /// Deletes a webhook-owned message with the scoped bearer credential.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_webhook_message(
        &self,
        webhook: Uuid,
        message: Uuid,
        token: &str,
    ) -> Result<(), Error> {
        self.anonymous_response(
            self.anonymous_route_request(
                &routes::DELETE_WEBHOOKS_WEBHOOK_ID_MESSAGES_MESSAGE_ID,
                &[webhook.to_string(), message.to_string()],
            )?
            .bearer_auth(token),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! Tests webhook creation response decoding without exercising transport.

    use super::CreatedWebhook;

    #[test]
    fn creation_response_requires_one_time_token() {
        assert!(serde_json::from_str::<CreatedWebhook>(r#"{"id":"id","name":"name"}"#).is_err());
    }
}
