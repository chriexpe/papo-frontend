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
        total_bytes: u64,
        root_cancel: tokio_util::sync::CancellationToken,
        image: Arc<StateCell<Arc<OutgoingSnapshot>>>,
    ) -> (Arc<Self>, ActiveProgressGuard) {
        let active = Arc::new(AtomicBool::new(true));
        let reporter = Arc::new(Self {
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
        if !self.active.load(Ordering::Acquire) || self.root_cancel.is_cancelled() {
            return;
        }
        publish_progress(&self.image, progress);
    }
}

pub(super) fn publish_progress(image: &StateCell<Arc<OutgoingSnapshot>>, progress: UploadProgress) {
    image.send_if_modified(|current| {
        if current.account != Some(progress.account)
            || progress.bytes_uploaded > progress.total_bytes
        {
            return false;
        }
        let still_sending = current
            .operations
            .iter()
            .any(|op| op.id == progress.operation_id && op.state == OperationState::Sending);
        if !still_sending {
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
mod tests {
    //! Pure deterministic coalescer tests using synthetic instants; no real clock or socket.
    use super::*;
    use crate::ServerAddress;

    fn account() -> AccountId {
        AccountId::new(
            ServerAddress::parse("https://example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        )
    }

    #[test]
    fn coalescer_bounds_many_small_chunks_and_never_exceeds_total() {
        let base = Instant::now();
        let total = 10_000_u64 * 1024;
        let mut coalescer = ProgressCoalescer::new(
            account(),
            Uuid::from_u128(2),
            total,
            base,
            MIN_PUBLISH_INTERVAL,
        );
        let mut published = Vec::new();
        for chunk in 0_u64..10_000 {
            // 10,000 chunks spread across 500 ms (50 us per chunk).
            let now = base + Duration::from_micros(chunk * 50);
            if let Some(progress) = coalescer.advance(1024, now) {
                published.push(progress);
            }
        }
        // 500 ms at 100 ms interval -> at most 5 intermediate ticks plus completion milestone.
        assert!(
            published.len() <= 6,
            "published {} updates",
            published.len()
        );
        assert_ne!(published.len(), 0);
        for pair in published.windows(2) {
            assert!(pair[0].bytes_uploaded < pair[1].bytes_uploaded);
        }
        let last = published.last().unwrap();
        assert_eq!(last.bytes_uploaded, total);
        assert_eq!(last.total_bytes, total);
        // Overshoot is clamped and never emits duplicate or > total values.
        assert!(
            coalescer
                .advance(4096, base + Duration::from_secs(10))
                .is_none()
        );
    }

    #[test]
    fn zero_delta_never_emits_duplicate_publication() {
        let base = Instant::now();
        let mut coalescer = ProgressCoalescer::new(
            account(),
            Uuid::from_u128(3),
            1024,
            base,
            MIN_PUBLISH_INTERVAL,
        );
        assert!(
            coalescer
                .advance(0, base + Duration::from_secs(1))
                .is_none()
        );
        let first = coalescer
            .advance(512, base + Duration::from_millis(100))
            .unwrap();
        assert_eq!(first.bytes_uploaded, 512);
        assert!(
            coalescer
                .advance(0, base + Duration::from_secs(2))
                .is_none()
        );
        let done = coalescer
            .advance(512, base + Duration::from_millis(101))
            .unwrap();
        assert_eq!(done.bytes_uploaded, 1024);
    }

    #[test]
    fn publish_progress_preserves_all_operation_arc_identities() {
        let acct = account();
        let op1 = Arc::new(super::super::Operation {
            id: Uuid::from_u128(10),
            channel_id: Uuid::from_u128(20),
            kind: Arc::new(super::super::OperationKind::Message(
                super::super::OutgoingMessage::default(),
            )),
            state: OperationState::Sending,
            attempts: 1,
            depends_on: None,
            failure: None,
            message_id: None,
            history_projected: false,
            message: None,
            created_at_us: 0,
        });
        let op2 = Arc::new(super::super::Operation {
            id: Uuid::from_u128(11),
            channel_id: Uuid::from_u128(21),
            kind: Arc::new(super::super::OperationKind::Message(
                super::super::OutgoingMessage::default(),
            )),
            state: OperationState::Queued,
            attempts: 0,
            depends_on: None,
            failure: None,
            message_id: None,
            history_projected: false,
            message: None,
            created_at_us: 0,
        });
        let cell = StateCell::new(Arc::new(OutgoingSnapshot {
            account: Some(acct),
            version: 1,
            operations: vec![Arc::clone(&op1), Arc::clone(&op2)],
            upload_progress: Some(UploadProgress::initial(acct, op1.id, 1000)),
            fault: None,
        }));
        publish_progress(
            &cell,
            UploadProgress {
                account: acct,
                operation_id: op1.id,
                bytes_uploaded: 400,
                total_bytes: 1000,
            },
        );
        let snap = cell.borrow().clone();
        assert_eq!(snap.version, 2);
        assert_eq!(snap.upload_progress.unwrap().bytes_uploaded, 400);
        assert!(Arc::ptr_eq(&snap.operations[0], &op1));
        assert!(Arc::ptr_eq(&snap.operations[1], &op2));
        // Non-Sending operations never accept stale progress updates.
        publish_progress(
            &cell,
            UploadProgress {
                account: acct,
                operation_id: op2.id,
                bytes_uploaded: 100,
                total_bytes: 1000,
            },
        );
        assert_eq!(cell.borrow().version, 2);
    }

    #[test]
    fn close_image_atomically_clears_progress_and_rejects_late_updates() {
        let acct = account();
        let op_id = Uuid::from_u128(42);
        let queue = super::super::AccountQueue::default();
        queue.image.send_if_modified(|current| {
            *current = Arc::new(OutgoingSnapshot {
                account: Some(acct),
                version: 3,
                operations: vec![Arc::new(super::super::Operation {
                    id: op_id,
                    channel_id: Uuid::from_u128(43),
                    kind: Arc::new(super::super::OperationKind::Message(
                        super::super::OutgoingMessage::default(),
                    )),
                    state: OperationState::Sending,
                    attempts: 1,
                    depends_on: None,
                    failure: None,
                    message_id: None,
                    history_projected: false,
                    message: None,
                    created_at_us: 0,
                })],
                upload_progress: Some(UploadProgress {
                    account: acct,
                    operation_id: op_id,
                    bytes_uploaded: 512,
                    total_bytes: 1024,
                }),
                fault: None,
            });
            true
        });
        let rx = queue.image.subscribe();
        queue.close_image();
        assert!(rx.has_changed().is_err());
        assert!(rx.borrow().upload_progress.is_none());
        assert_eq!(rx.borrow().version, 4);
        // Late progress ticks after close_image must never reinstate progress.
        publish_progress(
            &queue.image,
            UploadProgress {
                account: acct,
                operation_id: op_id,
                bytes_uploaded: 1024,
                total_bytes: 1024,
            },
        );
        assert!(rx.borrow().upload_progress.is_none());
        assert_eq!(rx.borrow().version, 4);
    }
}
