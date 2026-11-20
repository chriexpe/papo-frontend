//! Pure deterministic coalescer, attempt-fence and closure tests; no real clock or socket.

use super::*;
use crate::ServerAddress;
use tokio_util::sync::CancellationToken;

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    )
}

fn operation(id: u128, state: OperationState, attempts: u32) -> Arc<super::super::Operation> {
    Arc::new(super::super::Operation {
        id: Uuid::from_u128(id),
        channel_id: Uuid::from_u128(20),
        kind: Arc::new(super::super::OperationKind::Message(
            super::super::OutgoingMessage::default(),
        )),
        state,
        attempts,
        depends_on: None,
        failure: None,
        message_id: None,
        history_projected: false,
        message: None,
        created_at_us: 0,
    })
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
    let op1 = operation(10, OperationState::Sending, 1);
    let op2 = operation(11, OperationState::Queued, 0);
    let active = AtomicBool::new(true);
    let cancel = CancellationToken::new();
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
        1,
        &active,
        &cancel,
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
        0,
        &active,
        &cancel,
    );
    assert_eq!(cell.borrow().version, 2);
}

#[test]
fn stale_attempt_reporter_cannot_overwrite_subsequent_attempt_progress() {
    let acct = account();
    let op_id = Uuid::from_u128(30);
    let cancel = CancellationToken::new();
    let cell = StateCell::new(Arc::new(OutgoingSnapshot {
        account: Some(acct),
        version: 1,
        operations: vec![operation(30, OperationState::Sending, 1)],
        upload_progress: Some(UploadProgress::initial(acct, op_id, 1000)),
        fault: None,
    }));
    let (old_reporter, old_guard) =
        ProgressReporter::start(acct, op_id, 1, 1000, cancel.clone(), Arc::clone(&cell));

    // Attempt 1 finishes -> Queued -> Attempt 2 claims Sending (attempts: 2, progress reset to 0).
    cell.send_if_modified(|current| {
        *current = Arc::new(OutgoingSnapshot {
            account: Some(acct),
            version: 2,
            operations: vec![operation(30, OperationState::Sending, 2)],
            upload_progress: Some(UploadProgress::initial(acct, op_id, 1000)),
            fault: None,
        });
        true
    });

    // Interleaving A: even if attempt 1's active flag were still true when reaching the
    // publication boundary, its attempt fence (1 != 2) rejects overwriting attempt 2's 0 bytes.
    let stale_active = AtomicBool::new(true);
    publish_progress(
        &cell,
        UploadProgress {
            account: acct,
            operation_id: op_id,
            bytes_uploaded: 800,
            total_bytes: 1000,
        },
        1,
        &stale_active,
        &cancel,
    );
    assert_eq!(cell.borrow().version, 2);
    assert_eq!(cell.borrow().upload_progress.unwrap().bytes_uploaded, 0);

    // Interleaving B: dropping attempt 1's guard rejects publication at the boundary even for the
    // matching attempt number if active turned false before acquiring the StateCell lock.
    let inactive = AtomicBool::new(false);
    publish_progress(
        &cell,
        UploadProgress {
            account: acct,
            operation_id: op_id,
            bytes_uploaded: 500,
            total_bytes: 1000,
        },
        2,
        &inactive,
        &cancel,
    );
    assert_eq!(cell.borrow().version, 2);
    assert_eq!(cell.borrow().upload_progress.unwrap().bytes_uploaded, 0);

    // Current attempt 2 publishes cleanly.
    let active_2 = AtomicBool::new(true);
    publish_progress(
        &cell,
        UploadProgress {
            account: acct,
            operation_id: op_id,
            bytes_uploaded: 250,
            total_bytes: 1000,
        },
        2,
        &active_2,
        &cancel,
    );
    assert_eq!(cell.borrow().version, 3);
    assert_eq!(cell.borrow().upload_progress.unwrap().bytes_uploaded, 250);

    // Late completion tick on the old attempt 1 reporter cannot corrupt attempt 2.
    drop(old_guard);
    old_reporter.advance(1000);
    assert_eq!(cell.borrow().version, 3);
    assert_eq!(cell.borrow().upload_progress.unwrap().bytes_uploaded, 250);
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
            operations: vec![operation(42, OperationState::Sending, 1)],
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
    let active = AtomicBool::new(true);
    let cancel = CancellationToken::new();
    publish_progress(
        &queue.image,
        UploadProgress {
            account: acct,
            operation_id: op_id,
            bytes_uploaded: 1024,
            total_bytes: 1024,
        },
        1,
        &active,
        &cancel,
    );
    assert!(rx.borrow().upload_progress.is_none());
    assert_eq!(rx.borrow().version, 4);
}
