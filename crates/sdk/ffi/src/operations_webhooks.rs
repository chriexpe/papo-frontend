//! Webhook CRUD and scoped message authoring across FFI.
//! One-time creation tokens stay hidden behind non-`Debug` `FfiCreatedWebhook` until explicitly accessed.

use std::sync::Arc;

use papo_sdk::{
    OutgoingAttachment, OutgoingMessage,
    operations::webhooks::{
        CreateWebhookRequest, UpdateWebhookIdentityRequest, Webhook, WebhookMessage,
    },
};

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, parse_uuid, parse_uuid_list},
    messages::{FfiExternalAuthor, FfiMessage},
    operation_errors::{FfiOperationError, FfiRefreshStatus},
    operations_records::{
        FfiBatchUnitOutcome, FfiUnitReceipt, closed_operation_error, operation_session,
    },
    outgoing_payload::FfiEmbedInput,
};

/// Webhook definition metadata (contains no secret bearer token).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiWebhookWire {
    /// Webhook UUID string.
    pub id: Option<String>,
    /// Target channel UUID string.
    pub channel_id: Option<String>,
    /// Webhook name.
    pub name: Option<String>,
    /// Override display name.
    pub display_name: Option<String>,
    /// Creator user UUID string.
    pub created_by: Option<String>,
    /// Exact creation timestamp string.
    pub created_at: Option<String>,
    /// Revocation timestamp string if revoked.
    pub revoked_at: Option<String>,
}

impl From<&Webhook> for FfiWebhookWire {
    fn from(w: &Webhook) -> Self {
        Self {
            id: w.id.clone(),
            channel_id: w.channel_id.clone(),
            name: w.name.clone(),
            display_name: w.display_name.clone(),
            created_by: w.created_by.clone(),
            created_at: w.created_at.clone(),
            revoked_at: w.revoked_at.clone(),
        }
    }
}

/// Opaque single-use created webhook wrapper withholding the bearer token from `Debug`.
#[derive(uniffi::Object)]
pub struct FfiCreatedWebhook {
    /// Non-secret metadata record.
    webhook: FfiWebhookWire,
    /// One-time webhook token; never log or persist automatically.
    token: String,
}

#[uniffi::export]
impl FfiCreatedWebhook {
    /// Non-secret metadata for the created webhook.
    #[must_use]
    pub fn webhook(&self) -> FfiWebhookWire {
        self.webhook.clone()
    }

    /// Explicitly exposes the one-time webhook credential for user display or copy.
    #[must_use]
    pub fn expose_token(&self) -> String {
        self.token.clone()
    }
}

/// Receipt acknowledging webhook creation with non-`Debug` credential holder.
#[derive(Clone, uniffi::Record)]
pub struct FfiCreatedWebhookReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Opaque created webhook holding one-time credential.
    pub created: Arc<FfiCreatedWebhook>,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Inline attachment payload valid inside webhook authoring.
#[derive(Clone, uniffi::Record)]
pub struct FfiWebhookAttachmentInput {
    /// Safe basename without directories.
    pub filename: String,
    /// Attachment payload bytes.
    pub bytes: Vec<u8>,
}

/// Webhook execution message input (remote `reply_to` and inline bytes only).
#[derive(Clone, uniffi::Record)]
pub struct FfiWebhookMessageInput {
    /// Canonical message body text.
    pub content: String,
    /// Optional authoritative remote message UUID string to reply to.
    pub reply_to_message_id: Option<String>,
    /// Custom embeds.
    pub embeds: Vec<FfiEmbedInput>,
    /// Inline attachments.
    pub attachments: Vec<FfiWebhookAttachmentInput>,
    /// Optional bridged external author identity.
    pub external_author: Option<FfiExternalAuthor>,
    /// Optional webhook-scoped idempotency key.
    pub external_message_id: Option<String>,
}

/// Receipt carrying authoritative `Message` created or edited by a webhook.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiWebhookMessageReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Authoritative returned message.
    pub message: FfiMessage,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

#[uniffi::export]
impl FfiClient {
    /// Lists server webhooks after management authorization check.
    ///
    /// # Errors
    /// Returns rejection or session closure errors.
    pub async fn fetch_webhooks(
        &self,
        account: FfiAccountId,
    ) -> Result<Vec<FfiWebhookWire>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().webhooks().await?;
            Ok(g.value
                .webhooks
                .unwrap_or_default()
                .iter()
                .map(FfiWebhookWire::from)
                .collect())
        })
        .await
    }

    /// Creates a webhook in `channel_id` and returns its single-use bearer credential wrapper.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn create_webhook(
        &self,
        account: FfiAccountId,
        channel_id: String,
        name: String,
    ) -> Result<FfiCreatedWebhookReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let req = CreateWebhookRequest {
            channel_id: channel.to_string(),
            name,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().create_webhook(req).await?;
            Ok(FfiCreatedWebhookReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                created: Arc::new(FfiCreatedWebhook {
                    webhook: FfiWebhookWire::from(&r.value.webhook),
                    token: r.value.token,
                }),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Updates webhook display name/avatar overrides without changing ownership.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn update_webhook(
        &self,
        account: FfiAccountId,
        webhook_id: String,
        display_name: Option<String>,
        avatar_base64: Option<String>,
        avatar_format: Option<String>,
    ) -> Result<FfiWebhookWire, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let webhook = parse_uuid(&webhook_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let req = UpdateWebhookIdentityRequest {
            display_name,
            avatar: avatar_base64,
            avatar_format,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_webhook(webhook, req).await?;
            Ok(FfiWebhookWire::from(&r.value))
        })
        .await
    }

    /// Revokes one webhook credential.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn delete_webhook(
        &self,
        account: FfiAccountId,
        webhook_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let webhook = parse_uuid(&webhook_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().delete_webhook(webhook).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Revokes up to 100 webhooks sequentially without dropping partial results.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] for >100 items or invalid UUIDs.
    pub async fn delete_webhooks_batch(
        &self,
        account: FfiAccountId,
        webhook_ids: Vec<String>,
    ) -> Result<Vec<FfiBatchUnitOutcome>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let ids = parse_uuid_list(webhook_ids).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let results = session.operations().delete_webhooks_batch(ids).await?;
            Ok(results
                .into_iter()
                .map(FfiBatchUnitOutcome::from_sdk)
                .collect())
        })
        .await
    }

    /// Executes a scoped webhook message once using `webhook_token`.
    ///
    /// # Errors
    /// Returns invalid UUID/input, rejection, or `Unknown` dispatch failure.
    pub async fn execute_webhook(
        &self,
        account: FfiAccountId,
        webhook_id: String,
        webhook_token: String,
        message: FfiWebhookMessageInput,
    ) -> Result<FfiWebhookMessageReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let webhook = parse_uuid(&webhook_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let reply_to = message
            .reply_to_message_id
            .as_deref()
            .map(parse_uuid)
            .transpose()
            .map_err(|_| FfiOperationError::InvalidInput)?
            .map(papo_sdk::MessageTarget::Remote);
        let msg = WebhookMessage {
            message: OutgoingMessage {
                content: message.content,
                reply_to,
                embeds: message.embeds.into_iter().map(Into::into).collect(),
                attachments: message
                    .attachments
                    .into_iter()
                    .map(|a| OutgoingAttachment::from_bytes(a.filename, a.bytes))
                    .collect(),
                gif: None,
            },
            external_author: message.external_author.map(|ea| papo_sdk::ExternalAuthor {
                source: ea.source,
                id: ea.id,
                name: ea.name,
                avatar_url: ea.avatar_url,
                profile_url: ea.profile_url,
            }),
            external_message_id: message.external_message_id,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .execute_webhook(webhook, webhook_token, msg)
                .await?;
            Ok(FfiWebhookMessageReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                message: FfiMessage::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Edits a webhook-owned message using `webhook_token`.
    ///
    /// # Errors
    /// Returns invalid UUID/input, rejection, or `Unknown` dispatch outcome.
    pub async fn edit_webhook_message(
        &self,
        account: FfiAccountId,
        webhook_id: String,
        message_id: String,
        webhook_token: String,
        content: String,
        embeds: Option<Vec<FfiEmbedInput>>,
    ) -> Result<FfiWebhookMessageReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let webhook = parse_uuid(&webhook_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let sdk_embeds = embeds.map(|list| list.into_iter().map(Into::into).collect());
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .edit_webhook_message(webhook, message, webhook_token, content, sdk_embeds)
                .await?;
            Ok(FfiWebhookMessageReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                message: FfiMessage::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Deletes a webhook-owned message using `webhook_token`.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn delete_webhook_message(
        &self,
        account: FfiAccountId,
        webhook_id: String,
        message_id: String,
        webhook_token: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let webhook = parse_uuid(&webhook_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .delete_webhook_message(webhook, message, webhook_token)
                .await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }
}
