//! Public async outgoing/draft services and committed publications; network dispatch lives in dispatch.

use super::{
    Arc, ClientError, Draft, DraftBody, Operation, OperationKind, OperationState, OutgoingError,
    OutgoingMessage, OutgoingSnapshot, RetryDecision, Uuid,
};
use crate::{Session, UnixMicros};
use std::{future::Future, sync::atomic::Ordering};
use tokio::sync::watch;

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
                Ok(async {
                    service.check()?;
                    action(service).await
                }
                .await)
            })
            .await?
    }
    pub(super) async fn ensure(&self) -> Result<(), OutgoingError> {
        let queue = &self.session.inner.outgoing;
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
            {
                *operation = Arc::clone(old);
            }
        }
        drop(previous);
        let version = queue
            .image
            .borrow()
            .version
            .checked_add(1)
            .ok_or(OutgoingError::InvalidData)?;
        queue.image.send_replace(Arc::new(OutgoingSnapshot {
            account: Some(self.session.account()),
            version,
            operations,
            fault: None,
        }));
        Ok(())
    }
    /// Captures a committed baseline and a coalescing watch under the same state lock.
    /// Clone an image and release watch borrows before awaiting or reentering this service.
    /// # Errors
    /// Returns revoked account or durable decode/read failures.
    pub async fn subscribe(&self) -> Result<watch::Receiver<Arc<OutgoingSnapshot>>, OutgoingError> {
        self.run(|this| async move {
            this.ensure().await?;
            Ok(this.session.inner.outgoing.image.subscribe())
        })
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
        self.enqueue_inner(channel, local, kind, at, None).await
    }
    async fn enqueue_inner(
        &self,
        channel: Uuid,
        local: Uuid,
        kind: OperationKind,
        at: UnixMicros,
        revision: Option<u64>,
    ) -> Result<(), OutgoingError> {
        self.run(move |this| async move {
            this.ensure().await?;
            let _guard = this.session.inner.outgoing.serial.lock().await;
            this.check()?;
            this.session
                .client
                .inner
                .registry
                .outgoing_enqueue(this.session.clone(), channel, local, kind, at.0, revision)
                .await?;
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
        self.enqueue_inner(
            channel,
            local,
            OperationKind::Message(OutgoingMessage::default()),
            at,
            Some(revision),
        )
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
            let revision = this
                .session
                .client
                .inner
                .registry
                .draft_save(this.session.clone(), channel, body, at.0, expected)
                .await?;
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
