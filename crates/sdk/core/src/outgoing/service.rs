//! Public async outgoing/draft services and committed publications; network dispatch lives in dispatch.

use super::{
    Arc, ClientError, Draft, DraftBody, EnqueueSource, Operation, OperationKind, OperationState,
    OutgoingError, OutgoingSnapshot, RetryDecision, StagedAttachment, UploadProgress, Uuid,
};
use crate::{Session, UnixMicros};
use std::{future::Future, sync::atomic::Ordering};
use tokio::sync::watch;

pub(super) enum ProgressUpdate {
    Preserve,
    Set(Option<UploadProgress>),
}

/// An account-owned queue capability. Clones never retarget after account replacement.
#[derive(Clone)]
pub struct OutgoingService {
    pub(super) session: Session,
}
impl Session {
    /// Gets this account's durable outgoing service and starts its single bounded dispatcher.
    #[must_use]
    pub fn outgoing(&self) -> OutgoingService {
        super::dispatch::start(self.clone());
        OutgoingService {
            session: self.clone(),
        }
    }
}
impl OutgoingService {
    pub(super) fn check(&self) -> Result<(), OutgoingError> {
        if self.session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        if matches!(
            *self.session.inner.auth.borrow(),
            crate::AuthState::SignedOut | crate::AuthState::Expired
        ) {
            return Err(ClientError::SignedOut.into());
        }
        Ok(())
    }
    async fn run<
        T: Send + 'static,
        F: Future<Output = Result<T, OutgoingError>> + Send + 'static,
    >(
        &self,
        action: impl FnOnce(Self) -> F + Send + 'static,
    ) -> Result<T, OutgoingError> {
        let service = self.clone();
        self.session
            .client
            .dispatch(async move {
                if let Err(error) = service.check() {
                    return Ok(Err(error));
                }
                Ok(action(service).await)
            })
            .await?
    }
    pub(super) async fn ensure(&self) -> Result<(), OutgoingError> {
        let queue = &self.session.inner.outgoing;
        if queue.initialized.load(Ordering::Acquire) {
            return self.check();
        }
        let _guard = queue.serial.lock().await;
        self.check()?;
        if !queue.initialized.load(Ordering::Acquire) {
            self.collect().await?;
            self.publish().await?;
            queue.initialized.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub(super) async fn collect(&self) -> Result<(), OutgoingError> {
        self.session
            .client
            .inner
            .registry
            .outgoing_collect(self.session.clone())
            .await
    }
    pub(super) async fn publish(&self) -> Result<(), OutgoingError> {
        self.publish_with_progress(ProgressUpdate::Preserve).await
    }
    pub(super) async fn publish_with_progress(
        &self,
        progress: ProgressUpdate,
    ) -> Result<(), OutgoingError> {
        let queue = &self.session.inner.outgoing;
        let previous = queue.image.borrow().clone();
        let mut operations = self
            .session
            .client
            .inner
            .registry
            .outgoing_list(self.session.clone(), previous.clone())
            .await?;
        self.check()?;
        let old: std::collections::BTreeMap<_, _> =
            previous.operations.iter().map(|op| (op.id, op)).collect();
        for operation in &mut operations {
            if let Some(old) = old.get(&operation.id)
                && old.state == operation.state
                && old.attempts == operation.attempts
                && old.failure == operation.failure
                && old.message_id == operation.message_id
                && old.history_projected == operation.history_projected
            {
                *operation = Arc::clone(old);
            }
        }
        drop(previous);
        let account = self.session.account();
        let mut overflow = false;
        queue.image.send_if_modified(|current| {
            let Some(version) = current.version.checked_add(1) else {
                overflow = true;
                return false;
            };
            let upload_progress = match progress {
                ProgressUpdate::Set(value) => value,
                ProgressUpdate::Preserve => current.upload_progress.filter(|active| {
                    operations.iter().any(|op| {
                        op.id == active.operation_id && op.state == OperationState::Sending
                    })
                }),
            };
            *current = Arc::new(OutgoingSnapshot {
                account: Some(account),
                version,
                operations,
                upload_progress,
                fault: None,
            });
            true
        });
        if overflow {
            return Err(OutgoingError::InvalidData);
        }
        Ok(())
    }
    /// Streams an attachment source into this account's durable content-addressed staging without
    /// materializing the full file in memory.
    ///
    /// Enforces the 100 MiB per-attachment ceiling while copying, hashes incrementally, syncs the
    /// staged file before returning and removes partial files on failure. While the returned
    /// [`StagedAttachment`] handle stays alive in memory, background orphan collection preserves
    /// its blob; once dropped without a durable draft or operation reference (or after restart),
    /// the file remains eligible for collection.
    ///
    /// `reader` is driven synchronously on a tracked blocking worker, so a reader whose `read()`
    /// call blocks indefinitely (for example an unclosed pipe or stalled IPC stream) can delay
    /// account removal or client shutdown until `read()` returns; callers should supply bounded
    /// local file readers or sources that unblock promptly.
    /// # Errors
    /// Returns [`OutgoingError::InvalidInput`] above 100 MiB, [`OutgoingError::Source`] on reader
    /// failure, or account/storage failures.
    pub async fn stage_attachment<R>(&self, reader: R) -> Result<StagedAttachment, OutgoingError>
    where
        R: std::io::Read + Send + 'static,
    {
        self.run(move |this| async move {
            this.ensure().await?;
            this.session
                .client
                .inner
                .registry
                .stage_reader(&this.session, reader, Uuid::nil())
                .await
        })
        .await
    }
    /// Observes committed images and independent initialization faults without waiting for storage.
    /// Clone an image and release watch borrows before awaiting or reentering this service.
    /// # Errors
    /// Returns a revoked account; storage initialization failures arrive as a typed queue fault.
    pub async fn subscribe(&self) -> Result<watch::Receiver<Arc<OutgoingSnapshot>>, OutgoingError> {
        self.run(|this| async move { Ok(this.session.inner.outgoing.image.subscribe()) })
            .await
    }
    /// Enqueues an immutable intent in FULL insertion order, never ordered by wall-clock timestamps.
    /// Duplicate IDs are rejected rather than interpreted as retries.
    /// # Errors
    /// Returns invalid input/dependency, duplicate identity, account budget or durable failures.
    pub async fn enqueue(
        &self,
        channel: Uuid,
        local: Uuid,
        kind: OperationKind,
        at: UnixMicros,
    ) -> Result<(), OutgoingError> {
        self.enqueue_source(channel, local, EnqueueSource::Kind(kind), at, None)
            .await
    }
    /// Internal ephemeral-command admission. Already committed operations remain generation-independent.
    pub(crate) async fn enqueue_in_generation(
        &self,
        channel: Uuid,
        local: Uuid,
        kind: OperationKind,
        at: UnixMicros,
        generation: crate::Generation,
    ) -> Result<(), OutgoingError> {
        self.enqueue_source(
            channel,
            local,
            EnqueueSource::Kind(kind),
            at,
            Some(generation),
        )
        .await
    }
    async fn enqueue_source(
        &self,
        channel: Uuid,
        local: Uuid,
        source: EnqueueSource,
        at: UnixMicros,
        expected_generation: Option<crate::Generation>,
    ) -> Result<(), OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            this.check()?;
            if let Some(generation) = expected_generation {
                this.session.inner.epochs.validate(generation)?;
            }
            let result = this
                .session
                .client
                .inner
                .registry
                .outgoing_enqueue(
                    this.session.clone(),
                    channel,
                    local,
                    source,
                    at.0,
                    expected_generation,
                )
                .await;
            if let Err(error) = result {
                let _ = this.collect().await;
                return Err(error);
            }
            this.publish().await?;
            this.session.inner.outgoing.wake.notify_one();
            Ok(())
        })
        .await
    }
    /// Atomically captures this exact durable draft revision into a new message operation.
    /// The draft remains until Sent; a subsequent edited revision is never cleared by this send.
    /// # Errors
    /// Returns stale draft, invalid payload/dependency, duplicate ID, budget or durable failures.
    pub async fn enqueue_draft(
        &self,
        channel: Uuid,
        local: Uuid,
        revision: u64,
        at: UnixMicros,
    ) -> Result<(), OutgoingError> {
        self.enqueue_source(channel, local, EnqueueSource::Draft { revision }, at, None)
            .await
    }
    /// Restores this account/channel's durable draft, including immutable attachments.
    /// # Errors
    /// Returns account lifecycle or durable state failures.
    pub async fn draft(&self, channel: Uuid) -> Result<Option<Draft>, OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            this.session
                .client
                .inner
                .registry
                .draft_load(this.session.clone(), channel)
                .await
        })
        .await
    }
    /// FULL commits a draft and returns its new optimistic revision. Empty bodies clear its content.
    /// `expected` rejects concurrent editors; None explicitly replaces the current revision.
    /// # Errors
    /// Returns stale revision, invalid content or durable/account failures.
    pub async fn save_draft(
        &self,
        channel: Uuid,
        body: DraftBody,
        at: UnixMicros,
        expected: Option<u64>,
    ) -> Result<u64, OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            let revision = match this
                .session
                .client
                .inner
                .registry
                .draft_save(this.session.clone(), channel, body, at.0, expected)
                .await
            {
                Ok(revision) => revision,
                Err(error) => {
                    let _ = this.collect().await;
                    return Err(error);
                }
            };
            this.collect().await?;
            Ok(revision)
        })
        .await
    }
    /// Explicitly retries Failed work, or Unknown with `AcceptPossibleDuplicate`. Never runs automatically.
    /// # Errors
    /// Returns duplicate-risk acknowledgement, invalid transition, revoked or durable failures.
    pub async fn retry(&self, local: Uuid, decision: RetryDecision) -> Result<(), OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            let operation = this.operation(local)?;
            if operation.failure == Some(super::OperationFailure::Dependency) {
                return Err(OutgoingError::InvalidState);
            }
            if operation.state == OperationState::Unknown
                && decision != RetryDecision::AcceptPossibleDuplicate
            {
                return Err(OutgoingError::DuplicateRisk);
            }
            if !matches!(
                operation.state,
                OperationState::Failed | OperationState::Unknown
            ) {
                return Err(OutgoingError::InvalidState);
            }
            this.session
                .client
                .inner
                .registry
                .outgoing_transition(
                    this.session.clone(),
                    local,
                    operation.state,
                    OperationState::Queued,
                    None,
                    None,
                )
                .await?;
            this.publish().await?;
            this.session.inner.outgoing.wake.notify_one();
            Ok(())
        })
        .await
    }
    /// Discards an unresolved intent or dismisses a confirmed receipt; cannot cancel an in-flight send.
    /// Dependents fail explicitly rather than being retargeted or sent without their predecessor.
    /// # Errors
    /// Returns missing/currently Sending identity or account/storage failures.
    pub async fn discard(&self, local: Uuid) -> Result<(), OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            this.session
                .client
                .inner
                .registry
                .outgoing_discard(this.session.clone(), local)
                .await?;
            this.publish().await?;
            this.collect().await?;
            this.session.inner.outgoing.wake.notify_one();
            Ok(())
        })
        .await
    }
    fn operation(&self, id: Uuid) -> Result<Arc<Operation>, OutgoingError> {
        self.session
            .inner
            .outgoing
            .image
            .borrow()
            .operations
            .iter()
            .find(|op| op.id == id)
            .cloned()
            .ok_or(OutgoingError::NotFound)
    }
}
