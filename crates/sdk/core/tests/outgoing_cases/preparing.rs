//! Sending while a file is still copying: Preparing, then Queued and Sent; ✕, Retry, Discard,
//! a failed copy and a restart while preparing.

use super::*;
use papo_sdk::{AttachmentEdit, OperationFailure, PendingAttachment, StagingFailure};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Condvar, Mutex};

/// A source that blocks until opened, then yields its bytes (or fails).
#[derive(Clone)]
struct Gate(Arc<(Mutex<Option<bool>>, Condvar)>);

impl Gate {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(None), Condvar::new())))
    }
    fn open(&self, succeed: bool) {
        *self.0.0.lock().unwrap() = Some(succeed);
        self.0.1.notify_all();
    }
    fn reader(&self, bytes: Vec<u8>) -> GatedReader {
        GatedReader {
            gate: self.clone(),
            bytes: std::io::Cursor::new(bytes),
        }
    }
}

struct GatedReader {
    gate: Gate,
    bytes: std::io::Cursor<Vec<u8>>,
}

impl std::io::Read for GatedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let (lock, ready) = &*self.gate.0;
        let mut state = lock.lock().unwrap();
        while state.is_none() {
            state = ready.wait(state).unwrap();
        }
        if *state == Some(false) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "gone",
            ));
        }
        drop(state);
        std::io::Read::read(&mut self.bytes, buffer)
    }
}

fn with_pending(name: &str, pending: PendingAttachment) -> OperationKind {
    OperationKind::Message(OutgoingMessage {
        content: name.into(),
        attachments: vec![OutgoingAttachment::from_pending(name, pending)],
        ..Default::default()
    })
}

fn text(content: &str) -> OperationKind {
    OperationKind::Message(OutgoingMessage {
        content: content.into(),
        ..Default::default()
    })
}

#[test]
fn a_message_sent_while_its_file_copies_is_preparing_then_sends_in_order() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let bytes = vec![7_u8; 300 * 1024];
        let pending = queue.begin_staging(gate.reader(bytes.clone()))?;
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("late.bin", pending),
                UnixMicros(0),
            )
            .await?;
        queue
            .enqueue(channel, id(2), text("after the file"), UnixMicros(1))
            .await?;
        state(&session, id(1), OperationState::Preparing).await?;
        // The text behind it in the same channel waits.
        let queued = state(&session, id(2), OperationState::Queued).await?;
        assert_eq!(queued.state, OperationState::Queued);
        gate.open(true);
        state(&session, id(1), OperationState::Sent).await?;
        state(&session, id(2), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn the_uploaded_file_is_the_copied_one() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let bytes: Vec<u8> = (0..200_000_u32)
            .map(|n| u8::try_from(n % 253).unwrap())
            .collect();
        let pending = queue.begin_staging(gate.reader(bytes.clone()))?;
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("copied.bin", pending),
                UnixMicros(0),
            )
            .await?;
        gate.open(true);
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_eq!(upload.files[0].filename, "copied.bin");
        assert_eq!(
            upload.files[0].sha256,
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn cancel_while_preparing_fails_at_once_and_retry_sends_once_copied() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let pending = queue.begin_staging(gate.reader(vec![1; 64 * 1024]))?;
        queue.enqueue(channel, id(1), with_pending("x.bin", pending), UnixMicros(0)).await?;
        state(&session, id(1), OperationState::Preparing).await?;
        queue.cancel_upload(id(1)).await?;
        let failed = state(&session, id(1), OperationState::Failed).await?;
        assert_eq!(failed.failure, Some(OperationFailure::Cancelled));
        // Retry before the copy ends shows it preparing again; cancel again, then let it finish.
        queue.retry(id(1), RetryDecision::DefiniteFailure).await?;
        state(&session, id(1), OperationState::Preparing).await?;
        queue.cancel_upload(id(1)).await?;
        gate.open(true);
        // Once copied it stays Failed (Cancelled), now with its file, and Retry sends it.
        let mut rx = queue.subscribe().await?;
        loop {
            let resolved = rx.borrow_and_update().operations.iter().any(|op| {
                op.id == id(1)
                    && matches!(op.kind.as_ref(), OperationKind::Message(m) if m.attachments[0].staged().is_some())
            });
            if resolved {
                break;
            }
            rx.changed().await?;
        }
        queue.retry(id(1), RetryDecision::DefiniteFailure).await?;
        state(&session, id(1), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn discard_while_preparing_removes_it_and_stops_the_copy() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let pending = queue.begin_staging(gate.reader(vec![1; 64 * 1024]))?;
        let watcher = pending.clone();
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("x.bin", pending),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Preparing).await?;
        queue.discard(id(1)).await?;
        assert!(
            queue
                .subscribe()
                .await?
                .borrow()
                .operations
                .iter()
                .all(|op| op.id != id(1))
        );
        drop(watcher);
        gate.open(true);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_failed_copy_leaves_the_message_failed_and_discard_only() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let pending = queue.begin_staging(gate.reader(vec![1; 1024]))?;
        let watcher = pending.clone();
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("x.bin", pending),
                UnixMicros(0),
            )
            .await?;
        gate.open(false);
        let failed = state(&session, id(1), OperationState::Failed).await?;
        assert_eq!(failed.failure, Some(OperationFailure::AttachmentLost));
        assert!(matches!(
            watcher.staged().await,
            Err(StagingFailure::Unreadable(_))
        ));
        assert!(matches!(
            queue.retry(id(1), RetryDecision::DefiniteFailure).await,
            Err(OutgoingError::InvalidState)
        ));
        queue.discard(id(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_restart_while_preparing_brings_it_back_failed_with_its_text() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let gate = Gate::new();
    let account = f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let pending = queue.begin_staging(gate.reader(vec![1; 1024]))?;
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("x.bin", pending),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Preparing).await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    // Let the blocked copy end so shutdown can drain it.
    gate.open(false);
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        let failed = state(&session, id(1), OperationState::Failed).await?;
        assert_eq!(failed.failure, Some(OperationFailure::AttachmentLost));
        let OperationKind::Message(message) = failed.kind.as_ref() else {
            return Err("kind changed".into());
        };
        assert_eq!(message.content, "x.bin");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn an_edit_on_a_copying_file_is_applied_once_it_is_ready() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let pending = queue.begin_staging(gate.reader(b"%PDF-1.7 a document".to_vec()))?;
        let edit = AttachmentEdit {
            random_name: true,
            ..AttachmentEdit::default()
        };
        let attachment = OutgoingAttachment::from_pending("tax-2026.pdf", pending).with_edit(edit);
        queue
            .enqueue(
                channel,
                id(1),
                OperationKind::Message(OutgoingMessage {
                    attachments: vec![attachment],
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        gate.open(true);
        state(&session, id(1), OperationState::Sent).await?;
        let upload = f.server().take_received_upload().await.ok_or("no upload")?;
        assert_ne!(upload.files[0].filename, "tax-2026.pdf");
        assert_eq!(
            upload.files[0]
                .filename
                .rsplit_once('.')
                .map(|(_, extension)| extension),
            Some("pdf")
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_draft_does_not_take_a_file_still_copying() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        let pending = queue.begin_staging(gate.reader(vec![1; 16]))?;
        let saved = queue
            .save_draft(
                channel,
                DraftBody {
                    message: OutgoingMessage {
                        attachments: vec![OutgoingAttachment::from_pending("x.bin", pending)],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                UnixMicros(0),
                None,
            )
            .await;
        assert!(matches!(saved, Err(OutgoingError::InvalidInput)));
        gate.open(true);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_file_copied_before_send_goes_straight_to_queued() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let gate = Gate::new();
        gate.open(true);
        let pending = queue.begin_staging(gate.reader(vec![9; 32 * 1024]))?;
        // The copy has ended before Send: the resolver must not need the snapshot to have it.
        pending
            .staged()
            .await
            .map_err(|failure| format!("{failure:?}"))?;
        queue
            .enqueue(
                channel,
                id(1),
                with_pending("early.bin", pending),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
