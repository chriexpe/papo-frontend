//! Durable outgoing queue, attachment staging, drafts, upload progress, and explicit retry controls.
//! Never guesses retry safety for `Unknown` operations or bypasses durable state transitions.

use std::sync::Arc;

use papo_sdk::{DraftBody, Gif, OperationKind, OutgoingAttachment, OutgoingMessage, UnixMicros};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid, parse_uuid_list},
    outgoing_payload::{
        FfiDraft, FfiDraftBodyInput, FfiMessageTarget, FfiOutgoingAttachmentInput,
        FfiOutgoingMessageInput, FfiStagedAttachment,
    },
    outgoing_state::{
        FfiOperationKindInput, FfiOutgoingListener, FfiRetryDecision, map_outgoing_snapshot,
    },
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
    timeline_errors::FfiOutgoingError,
};

/// Converts input attachments + embeds into SDK `OutgoingMessage`.
fn build_outgoing_message(
    msg: FfiOutgoingMessageInput,
) -> Result<OutgoingMessage, FfiOutgoingError> {
    let reply_to = msg.reply_to.map(FfiMessageTarget::into_sdk).transpose()?;
    let embeds = msg.embeds.into_iter().map(Into::into).collect();
    let attachments = msg
        .attachments
        .into_iter()
        .map(|att| match att {
            FfiOutgoingAttachmentInput::Bytes { filename, bytes } => {
                OutgoingAttachment::from_bytes(filename, bytes)
            }
            FfiOutgoingAttachmentInput::Staged { filename, staged } => {
                OutgoingAttachment::from_staged(filename, staged.inner.clone())
            }
        })
        .collect();
    let gif = msg.gif.map(Gif::from);
    Ok(OutgoingMessage {
        content: msg.content,
        reply_to,
        embeds,
        attachments,
        gif,
    })
}

#[uniffi::export]
impl FfiClient {
    /// Streams an OS file into account outgoing staging off Tokio worker threads.
    ///
    /// Returns a pinned handle that keeps the staged blob protected until saved in a draft,
    /// enqueued, or dropped.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::Source`] if opening/reading fails, or input/budget/storage errors.
    pub async fn stage_attachment_file(
        &self,
        account: FfiAccountId,
        source_path: String,
    ) -> Result<Arc<FfiStagedAttachment>, FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let file = tokio::task::spawn_blocking(move || std::fs::File::open(source_path))
                    .await
                    .map_err(|_| FfiOutgoingError::Source)?
                    .map_err(|_| FfiOutgoingError::Source)?;
                let staged = service.stage_attachment(file).await?;
                Ok(Arc::new(FfiStagedAttachment { inner: staged }))
            },
        )
        .await
    }

    /// Subscribes to atomic outgoing queue and upload progress snapshots for `account`.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::Session`] if account missing or worker start fails.
    pub async fn subscribe_outgoing(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiOutgoingListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let service = session.outgoing();
        let runtime = self.runtime.clone();
        let tasks = self.tasks.clone();
        let shutdown = self.shutdown.clone();
        let callbacks = self.callbacks.clone();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let rx = service.subscribe().await?;
                let (baseline, stream) = watch_stream(rx);
                let l_snap = Arc::clone(&listener);
                let l_upd = Arc::clone(&listener);
                let l_end = listener;
                spawn_stream_subscription(
                    &runtime,
                    &tasks,
                    &shutdown,
                    callbacks,
                    map_outgoing_snapshot(&baseline),
                    stream,
                    move |snap| {
                        l_snap.on_snapshot(snap);
                        true
                    },
                    move |snap| {
                        l_upd.on_update(map_outgoing_snapshot(&snap));
                        true
                    },
                    move || l_end.on_closed(),
                )
                .map_err(|err| FfiOutgoingError::Session { error: err })
            },
        )
        .await
    }

    /// Enqueues a durable message, edit, delete, or reaction operation with `synchronous=FULL`.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError`].
    pub async fn enqueue_operation(
        &self,
        account: FfiAccountId,
        channel_id: String,
        local_id: String,
        kind: FfiOperationKindInput,
        created_at_us: i64,
    ) -> Result<(), FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let local = parse_uuid(&local_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let sdk_kind = match kind {
            FfiOperationKindInput::Message { message } => {
                OperationKind::Message(build_outgoing_message(message)?)
            }
            FfiOperationKindInput::Edit {
                target,
                content,
                embeds,
            } => OperationKind::Edit {
                target: target.into_sdk()?,
                content,
                embeds: embeds.map(|list| list.into_iter().map(Into::into).collect()),
            },
            FfiOperationKindInput::Delete { target } => OperationKind::Delete {
                target: target.into_sdk()?,
            },
            FfiOperationKindInput::Reaction {
                target,
                emoji_id,
                unicode,
                add,
            } => OperationKind::Reaction {
                target: target.into_sdk()?,
                emoji_id: emoji_id
                    .map(|s| parse_uuid(&s).map_err(|_| FfiOutgoingError::InvalidInput))
                    .transpose()?,
                unicode,
                add,
            },
        };
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                service
                    .enqueue(channel, local, sdk_kind, UnixMicros(created_at_us))
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Loads this account/channel's FULL-committed composer draft if one exists.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError`].
    pub async fn load_draft(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<Option<FfiDraft>, FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let draft = service.draft(channel).await?;
                Ok(draft.as_ref().map(Into::into))
            },
        )
        .await
    }

    /// Saves a composer draft atomically with optional optimistic concurrency `expected_revision`.
    ///
    /// Returns the newly assigned durable revision number.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::StaleDraft`], input, budget, or storage errors.
    pub async fn save_draft(
        &self,
        account: FfiAccountId,
        channel_id: String,
        body: FfiDraftBodyInput,
        updated_at_us: i64,
        expected_revision: Option<u64>,
    ) -> Result<u64, FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let mentions =
            parse_uuid_list(body.mention_user_ids).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let sdk_body = DraftBody {
            message: build_outgoing_message(body.message)?,
            mentions,
        };
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                service
                    .save_draft(
                        channel,
                        sdk_body,
                        UnixMicros(updated_at_us),
                        expected_revision,
                    )
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Atomically promotes an exact draft revision into the durable send queue.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::StaleDraft`] if draft changed, or input/budget/storage errors.
    pub async fn enqueue_draft(
        &self,
        account: FfiAccountId,
        channel_id: String,
        local_id: String,
        revision: u64,
        created_at_us: i64,
    ) -> Result<(), FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let local = parse_uuid(&local_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                service
                    .enqueue_draft(channel, local, revision, UnixMicros(created_at_us))
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Explicitly retries an unresolved operation with user confirmation decision.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::DuplicateRisk`] if retrying `Unknown` with `DefiniteFailure`.
    pub async fn retry_outgoing(
        &self,
        account: FfiAccountId,
        local_id: String,
        decision: FfiRetryDecision,
    ) -> Result<(), FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let local = parse_uuid(&local_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                service
                    .retry(local, decision.into())
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Discards a queued/failed/unknown intent or dismisses a confirmed receipt cleanly.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError`].
    pub async fn discard_outgoing(
        &self,
        account: FfiAccountId,
        local_id: String,
    ) -> Result<(), FfiOutgoingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiOutgoingError::Session { error: err })?;
        let local = parse_uuid(&local_id).map_err(|_| FfiOutgoingError::InvalidInput)?;
        let service = session.outgoing();
        self.run_scoped(
            || FfiOutgoingError::Session {
                error: FfiClientError::Closed,
            },
            async move { service.discard(local).await.map_err(Into::into) },
        )
        .await
    }
}
