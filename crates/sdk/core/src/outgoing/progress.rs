//! Ephemeral in-memory upload progress coalescing and snapshot publication; never persisted to SQLite.

use super::{AccountId, OperationState, OutgoingSnapshot, Uuid};
use crate::observable::StateCell;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Minimum interval between intermediate upload progress publications per sending attempt.
pub(super) const MIN_PUBLISH_INTERVAL: Duration = Duration::from_millis(100);

/// Ephemeral in-memory upload progress for one actively dispatching attachment operation.
///
/// `bytes_uploaded` reports attachment payload bytes consumed by the HTTP transport
/// (excluding multipart boundary/header framing) during the current `Sending` attempt,
/// monotonically bounded by `total_bytes`.
///
/// Reaching `bytes_uploaded == total_bytes` means only that the transport consumed the stream;
/// it never proves server receipt or acceptance. Only a confirmed [`OperationState::Sent`]
/// transition after a valid backend response establishes delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadProgress {
    /// Owning account; matches the enclosing [`OutgoingSnapshot::account`].
    pub account: AccountId,
    /// Local operation identity currently in [`OperationState::Sending`].
    pub operation_id: Uuid,
    /// Attachment payload bytes consumed by the transport in this attempt.
    pub bytes_uploaded: u64,
    /// Total attachment payload bytes across all attachments in this operation.
    pub total_bytes: u64,
}

impl UploadProgress {
    pub(super) const fn initial(account: AccountId, operation_id: Uuid, total_bytes: u64) -> Self {
        Self {
            account,
            operation_id,
            bytes_uploaded: 0,
            total_bytes,
        }
    }
}

pub(super) struct ProgressCoalescer {
    account: AccountId,
    operation_id: Uuid,
    total_bytes: u64,
    bytes_uploaded: u64,
    last_published_bytes: u64,
    last_published_at: Instant,
    min_interval: Duration,
}

impl ProgressCoalescer {
    pub(super) fn new(
        account: AccountId,
        operation_id: Uuid,
        total_bytes: u64,
        started_at: Instant,
        min_interval: Duration,
    ) -> Self {
        Self {
            account,
            operation_id,
            total_bytes,
            bytes_uploaded: 0,
            last_published_bytes: 0,
            last_published_at: started_at,
            min_interval,
        }
    }

    /// Advances consumed bytes and returns a progress snapshot only when monotonic and permitted
    /// by the coalescing interval (or upon reaching `total_bytes`).
    pub(super) fn advance(&mut self, delta: u64, now: Instant) -> Option<UploadProgress> {
        self.bytes_uploaded = self
            .bytes_uploaded
            .saturating_add(delta)
            .min(self.total_bytes);
        if self.bytes_uploaded <= self.last_published_bytes {
            return None;
        }
        let completed = self.bytes_uploaded == self.total_bytes;
        let elapsed = now.saturating_duration_since(self.last_published_at);
        if !completed && elapsed < self.min_interval {
            return None;
        }
        self.last_published_bytes = self.bytes_uploaded;
        self.last_published_at = now;
        Some(UploadProgress {
            account: self.account,
            operation_id: self.operation_id,
            bytes_uploaded: self.bytes_uploaded,
            total_bytes: self.total_bytes,
        })
    }
}

pub(super) struct ProgressReporter {
    attempt: u32,
    coalescer: Mutex<ProgressCoalescer>,
    active: Arc<AtomicBool>,
    root_cancel: tokio_util::sync::CancellationToken,
    image: Arc<StateCell<Arc<OutgoingSnapshot>>>,
}

pub(super) struct ActiveProgressGuard {
    active: Arc<AtomicBool>,
}

impl Drop for ActiveProgressGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

impl ProgressReporter {
    pub(super) fn start(
        account: AccountId,
        operation_id: Uuid,
        attempt: u32,
        total_bytes: u64,
        root_cancel: tokio_util::sync::CancellationToken,
        image: Arc<StateCell<Arc<OutgoingSnapshot>>>,
    ) -> (Arc<Self>, ActiveProgressGuard) {
        let active = Arc::new(AtomicBool::new(true));
        let reporter = Arc::new(Self {
            attempt,
            coalescer: Mutex::new(ProgressCoalescer::new(
                account,
                operation_id,
                total_bytes,
                Instant::now(),
                MIN_PUBLISH_INTERVAL,
            )),
            active: Arc::clone(&active),
            root_cancel,
            image,
        });
        (reporter, ActiveProgressGuard { active })
    }

    pub(super) fn advance(&self, delta: u64) {
        if !self.active.load(Ordering::Acquire) || self.root_cancel.is_cancelled() {
            return;
        }
        let next = {
            let mut coalescer = self
                .coalescer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            coalescer.advance(delta, Instant::now())
        };
        let Some(progress) = next else {
            return;
        };
        publish_progress(
            &self.image,
            progress,
            self.attempt,
            &self.active,
            &self.root_cancel,
        );
    }
}

pub(super) fn publish_progress(
    image: &StateCell<Arc<OutgoingSnapshot>>,
    progress: UploadProgress,
    expected_attempt: u32,
    active: &AtomicBool,
    root_cancel: &tokio_util::sync::CancellationToken,
) {
    image.send_if_modified(|current| {
        if !active.load(Ordering::Acquire)
            || root_cancel.is_cancelled()
            || current.account != Some(progress.account)
            || progress.bytes_uploaded > progress.total_bytes
        {
            return false;
        }
        let same_sending_attempt = current.operations.iter().any(|op| {
            op.id == progress.operation_id
                && op.state == OperationState::Sending
                && op.attempts == expected_attempt
        });
        if !same_sending_attempt {
            return false;
        }
        if let Some(existing) = current.upload_progress
            && existing.operation_id == progress.operation_id
            && progress.bytes_uploaded <= existing.bytes_uploaded
        {
            return false;
        }
        let Some(version) = current.version.checked_add(1) else {
            return false;
        };
        // Clones only the outer snapshot and Arc handles; no Operation is copied or replaced.
        let mut updated = (**current).clone();
        updated.version = version;
        updated.upload_progress = Some(progress);
        *current = Arc::new(updated);
        true
    });
}

#[cfg(test)]
mod tests;
