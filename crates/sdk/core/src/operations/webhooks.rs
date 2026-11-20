//! Webhook management and scoped authoring; credentials stay transient and foreign identity stays explicit.

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::Generated;
use crate::{MessageTarget, OutgoingMessage, Uuid};
pub use papo_api::{
    administration::CreatedWebhook,
    wire::{
        CreateWebhookRequest, ExternalAuthor, UpdateWebhookIdentityRequest, Webhook,
        WebhookListResponse,
    },
};
use papo_api::{
    administration::{WebhookAttachment, WebhookMessageRequest},
    wire::{EmbedInput, Message, UpdateMessageRequest},
};

/// Typed foreign authoring payload. No bridge-user ID is inferred from external identity.
#[derive(Clone, Default)]
pub struct WebhookMessage {
    /// Text supplied verbatim to the backend.
    pub message: OutgoingMessage,
    /// Optional foreign provider identity, never a Papo member identity.
    pub external_author: Option<ExternalAuthor>,
    /// Backend webhook-scoped idempotency identity; the SDK never retries automatically.
    pub external_message_id: Option<String>,
}

impl WebhookMessage {
    fn wire(self) -> Result<WebhookMessageRequest, OperationError> {
        self.message
            .validate()
            .map_err(|_| OperationError::InvalidInput)?;
        let reply_to = match self.message.reply_to {
            Some(MessageTarget::Remote(id)) => Some(id.to_string()),
            Some(MessageTarget::Local(_)) => return Err(OperationError::InvalidInput),
            None => None,
        };
        let external_author = self
            .external_author
            .map(|author| serde_json::to_string(&author))
            .transpose()
            .map_err(|_| OperationError::InvalidInput)?;
        let embeds = if self.message.embeds.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&self.message.embeds)
                    .map_err(|_| OperationError::InvalidInput)?,
            )
        };
        let attachments = self
            .message
            .attachments
            .into_iter()
            .map(|file| {
                let bytes = file
                    .inline_bytes()
                    .ok_or(OperationError::InvalidInput)?
                    .to_vec();
                Ok(WebhookAttachment {
                    filename: file.filename,
                    bytes,
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        Ok(WebhookMessageRequest {
            content: Some(self.message.content),
            external_message_id: self.external_message_id,
            external_author,
            reply_to,
            embeds,
            attachments,
        })
    }
}

impl Operations {
    /// Deletes at most 100 webhooks once each, retaining every item result in input order.
    /// A rejected or ambiguous request is never retried. Reconnect ends admission
    /// for remaining items rather than silently continuing in another generation.
    /// # Errors
    /// Returns `InvalidInput` when the batch exceeds 100 items.
    pub async fn delete_webhooks_batch(
        &self,
        items: Vec<Uuid>,
    ) -> Result<Vec<Result<OperationReceipt<()>, OperationError>>, OperationError> {
        if items.len() > 100 {
            return Err(OperationError::InvalidInput);
        }
        let operations = self.for_batch();
        let mut results = Vec::with_capacity(items.len());
        for webhook in items {
            results.push(operations.delete_webhook(webhook).await);
        }
        Ok(results)
    }

    /// Lists active webhooks after backend management authorization.
    /// # Errors
    /// Returns transport, authorization or stale-generation errors.
    pub async fn webhooks(&self) -> Result<Generated<WebhookListResponse>, OperationError> {
        self.read(|rest| async move { rest.webhooks().await }).await
    }

    /// Creates one webhook. The returned credential has no Debug/Serialize implementation.
    /// Its one-time token is caller-owned secret material; never log or cache it.
    /// # Errors
    /// Returns backend rejection or an ambiguous outcome; no automatic retry occurs.
    pub async fn create_webhook(
        &self,
        request: CreateWebhookRequest,
    ) -> Result<OperationReceipt<CreatedWebhook>, OperationError> {
        self.mutate(
            move |rest| async move { rest.create_webhook(&request).await },
            Refresh::None,
        )
        .await
    }

    /// Updates nickname/avatar without changing webhook ownership or foreign author identity.
    /// Missing fields preserve values; empty nickname/avatar removes that override.
    /// # Errors
    /// Returns backend rejection, stale identity or an ambiguous dispatch outcome.
    pub async fn update_webhook(
        &self,
        webhook: Uuid,
        request: UpdateWebhookIdentityRequest,
    ) -> Result<OperationReceipt<Webhook>, OperationError> {
        self.mutate(
            move |rest| async move { rest.update_webhook(webhook, &request).await },
            Refresh::None,
        )
        .await
    }

    /// Revokes the scoped credential after server authorization.
    /// # Errors
    /// Returns backend rejection, stale identity or an ambiguous dispatch outcome.
    pub async fn delete_webhook(
        &self,
        webhook: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_webhook(webhook).await },
            Refresh::None,
        )
        .await
    }

    /// Sends once with only the supplied scoped credential, never the account bearer.
    /// Foreign identity and authoritative embeds are returned unchanged. This does
    /// not create an ordinary user outgoing operation or grant timeline coverage.
    /// # Errors
    /// Returns invalid input, backend rejection or an ambiguous dispatch outcome.
    pub async fn execute_webhook(
        &self,
        webhook: Uuid,
        token: String,
        request: WebhookMessage,
    ) -> Result<OperationReceipt<Message>, OperationError> {
        let request = request.wire()?;
        self.mutate(
            move |rest| async move { rest.execute_webhook(webhook, &token, &request).await },
            Refresh::None,
        )
        .await
    }

    /// Replaces webhook-owned message text/custom embeds with its scoped credential.
    /// The backend enforces webhook ownership; cached member privileges are irrelevant.
    /// # Errors
    /// Returns backend rejection, invalid input or an ambiguous dispatch outcome.
    pub async fn edit_webhook_message(
        &self,
        webhook: Uuid,
        message: Uuid,
        token: String,
        content: String,
        embeds: Option<Vec<EmbedInput>>,
    ) -> Result<OperationReceipt<Message>, OperationError> {
        let checked = OutgoingMessage {
            content: content.clone(),
            ..OutgoingMessage::default()
        };
        checked
            .validate()
            .map_err(|_| OperationError::InvalidInput)?;
        let request = UpdateMessageRequest { content, embeds };
        self.mutate(
            move |rest| async move {
                rest.edit_webhook_message(webhook, message, &token, &request)
                    .await
            },
            Refresh::None,
        )
        .await
    }

    /// Deletes only a message owned by the authenticated webhook.
    /// # Errors
    /// Returns backend rejection, stale identity or an ambiguous dispatch outcome.
    pub async fn delete_webhook_message(
        &self,
        webhook: Uuid,
        message: Uuid,
        token: String,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_webhook_message(webhook, message, &token).await },
            Refresh::None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Pure authoring tests; backend permission/ownership checks remain authoritative.
    use super::*;

    #[test]
    fn local_dependency_cannot_escape_into_webhook_authoring() {
        let request = WebhookMessage {
            message: OutgoingMessage {
                reply_to: Some(MessageTarget::Local(Uuid::nil())),
                ..OutgoingMessage::default()
            },
            ..WebhookMessage::default()
        };
        assert!(matches!(request.wire(), Err(OperationError::InvalidInput)));
    }

    #[test]
    fn foreign_identity_remains_explicit_and_attachment_names_are_validated() {
        let author = ExternalAuthor {
            source: Some("fixture".into()),
            id: Some("foreign".into()),
            ..ExternalAuthor::default()
        };
        let request = WebhookMessage {
            external_author: Some(author),
            ..WebhookMessage::default()
        }
        .wire()
        .unwrap();
        let author: ExternalAuthor =
            serde_json::from_str(request.external_author.as_deref().unwrap()).unwrap();
        assert_eq!(author.id.as_deref(), Some("foreign"));
        let request = WebhookMessage {
            message: OutgoingMessage {
                attachments: vec![crate::OutgoingAttachment::from_bytes("../private", vec![])],
                ..OutgoingMessage::default()
            },
            ..WebhookMessage::default()
        };
        assert!(matches!(request.wire(), Err(OperationError::InvalidInput)));
    }
}
