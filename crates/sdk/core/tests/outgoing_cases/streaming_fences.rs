//! Attachment staging limits, live-handle pins, orphan collection, account removal and dispatch fences.

use super::streaming::{MAX_BYTES, MIB, PatternReader, pattern_sha256};
use super::*;
use papo_sdk::{DataRoot, OutgoingDispatchPhase, OutgoingDispatchProbe};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

struct PauseProbe {
    target_id: Uuid,
    target_phase: OutgoingDispatchPhase,
    hit: Mutex<bool>,
    reached: Notify,
    release: Notify,
}

impl PauseProbe {
    fn new(target_id: Uuid, target_phase: OutgoingDispatchPhase) -> Arc<Self> {
        Arc::new(Self {
            target_id,
            target_phase,
            hit: Mutex::new(false),
            reached: Notify::new(),
            release: Notify::new(),
        })
    }

    async fn wait_reached(&self) {
        let notified = self.reached.notified();
        if *self.hit.lock().unwrap() {
            return;
        }
        notified.await;
    }

    fn release(&self) {
        self.release.notify_waiters();
    }
}

impl OutgoingDispatchProbe for PauseProbe {
    fn reached(
        &self,
        id: Uuid,
        phase: OutgoingDispatchPhase,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if id == self.target_id && phase == self.target_phase {
                let release = self.release.notified();
                {
                    let mut hit = self.hit.lock().unwrap();
                    if *hit {
                        return;
                    }
                    *hit = true;
                }
                self.reached.notify_waiters();
                release.await;
            }
        })
    }
}

#[test]
fn staging_enforces_limits_errors_and_cross_account_isolation() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (a, channel) = signed_in(&f).await?;
        let auth = f.client().handle().auth(a.account().server())?;
        register(&auth, "stream-other").await?;
        let b = login(&auth, "stream-other").await?;
        fixture::online(&b).await?;
        let qa = a.outgoing();
        let qb = b.outgoing();

        // 100 MiB + 1 byte is rejected without leaving partial files.
        assert!(matches!(
            qa.stage_attachment(PatternReader::new(MAX_BYTES + 1, 0x01))
                .await,
            Err(OutgoingError::InvalidInput)
        ));
        // Mid-stream read error is reported as Source and removes partial file.
        assert!(matches!(
            qa.stage_attachment(PatternReader::failing(512 * 1024, 0x02, 128 * 1024))
                .await,
            Err(OutgoingError::Source(io::ErrorKind::BrokenPipe))
        ));
        let staging_a = DataRoot::new(f.profile.path()).outgoing_files(a.account());
        if staging_a.exists() {
            assert_eq!(std::fs::read_dir(&staging_a)?.count(), 0);
        }

        // Two 60 MiB staged files exceed the 100 MiB per-message total limit.
        let s1 = qa
            .stage_attachment(PatternReader::new(60 * MIB, 0x31))
            .await?;
        let s2 = qa
            .stage_attachment(PatternReader::new(60 * MIB, 0x32))
            .await?;
        assert!(matches!(
            qa.enqueue(
                channel,
                id(30),
                OperationKind::Message(OutgoingMessage {
                    attachments: vec![
                        OutgoingAttachment::from_staged("a.bin", s1.clone()),
                        OutgoingAttachment::from_staged("b.bin", s2),
                    ],
                    ..Default::default()
                }),
                UnixMicros(30),
            )
            .await,
            Err(OutgoingError::InvalidInput)
        ));
        drop(s1);

        // More than 10 attachments is rejected even when referencing one small staged file.
        let small = qa.stage_attachment(PatternReader::new(16, 0x44)).await?;
        assert!(matches!(
            qa.enqueue(
                channel,
                id(31),
                OperationKind::Message(OutgoingMessage {
                    attachments: (0..11)
                        .map(|i| OutgoingAttachment::from_staged(
                            format!("f{i}.bin"),
                            small.clone()
                        ))
                        .collect(),
                    ..Default::default()
                }),
                UnixMicros(31),
            )
            .await,
            Err(OutgoingError::InvalidInput)
        ));

        // Account B cannot enqueue Account A's StagedAttachment.
        assert!(matches!(
            qb.enqueue(
                channel,
                id(32),
                OperationKind::Message(OutgoingMessage {
                    attachments: vec![OutgoingAttachment::from_staged("stolen.bin", small)],
                    ..Default::default()
                }),
                UnixMicros(32),
            )
            .await,
            Err(OutgoingError::NotFound)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn live_staged_attachment_survives_collection_and_dropped_unreferenced_blob_is_reclaimed() -> Result
{
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (a, channel) = signed_in(&f).await?;
        let qa = a.outgoing();
        let staging_a = DataRoot::new(f.profile.path()).outgoing_files(a.account());

        // A live StagedAttachment handle survives concurrent collection and can still be enqueued.
        let live = qa.stage_attachment(PatternReader::new(32, 0x55)).await?;
        let live_path = staging_a.join(live.digest_hex());
        qa.save_draft(channel, DraftBody::default(), UnixMicros(33), None)
            .await?;
        assert!(
            live_path.exists(),
            "held StagedAttachment must survive background orphan collection"
        );
        qa.enqueue(
            channel,
            id(34),
            OperationKind::Message(OutgoingMessage {
                content: "pinned-send".into(),
                attachments: vec![OutgoingAttachment::from_staged("ok.bin", live)],
                ..Default::default()
            }),
            UnixMicros(34),
        )
        .await?;
        state(&a, id(34), OperationState::Sent).await?;

        // Dropping an unreferenced StagedAttachment allows orphan collection to reclaim its blob.
        let orphan = qa.stage_attachment(PatternReader::new(48, 0x66)).await?;
        let orphan_path = staging_a.join(orphan.digest_hex());
        assert!(orphan_path.exists());
        drop(orphan);
        qa.save_draft(channel, DraftBody::default(), UnixMicros(35), None)
            .await?;
        assert!(
            !orphan_path.exists(),
            "dropped unreferenced staged blob must be reclaimed by collection"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn account_removal_during_inflight_streamed_upload_drains_cleanly_and_removes_staging() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let account = session.account();
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(4 * MIB, 0x77))
            .await?;
        let mut hold = f
            .server()
            .hold_message_upload_after_bytes(256 * 1024)
            .await?;
        let rx = queue.subscribe().await?;
        queue
            .enqueue(
                channel,
                id(40),
                OperationKind::Message(OutgoingMessage {
                    content: "remove-inflight".into(),
                    attachments: vec![OutgoingAttachment::from_staged("inflight.bin", staged)],
                    ..Default::default()
                }),
                UnixMicros(40),
            )
            .await?;
        hold.wait_started().await?;
        f.client().handle().remove_account(account).await?;
        assert!(rx.has_changed().is_err());
        assert!(
            rx.borrow().upload_progress.is_none(),
            "account closure must clear ephemeral upload progress"
        );
        assert!(
            !DataRoot::new(f.profile.path())
                .outgoing_files(account)
                .exists(),
            "account removal must remove the staging directory after draining upload readers"
        );
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn progress_clears_on_unknown_and_never_persists_across_restart() -> Result {
    let mut f = Fixture::new(Config {
        rest: vec![RestStep {
            method: "POST".into(),
            path: "/messages".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 500,
                slug: "internal".into(),
            }),
        }],
        ..Default::default()
    })?;
    let (account, channel) = f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(128 * 1024, 0x88))
            .await?;
        queue
            .enqueue(
                channel,
                id(50),
                OperationKind::Message(OutgoingMessage {
                    content: "unknown-progress".into(),
                    attachments: vec![OutgoingAttachment::from_staged("u.bin", staged)],
                    ..Default::default()
                }),
                UnixMicros(50),
            )
            .await?;
        let unknown = state(&session, id(50), OperationState::Unknown).await?;
        assert!(queue.subscribe().await?.borrow().upload_progress.is_none());
        let OperationKind::Message(body) = unknown.kind.as_ref() else {
            return Err("expected message".into());
        };
        assert!(body.attachments[0].inline_bytes().is_none());
        assert_eq!(
            body.attachments[0].staged().unwrap().size_bytes(),
            128 * 1024
        );
        Ok::<_, Box<dyn std::error::Error>>((session.account(), channel))
    })??;

    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        let queue = session.outgoing();
        let op = state(&session, id(50), OperationState::Unknown).await?;
        assert!(queue.subscribe().await?.borrow().upload_progress.is_none());
        let OperationKind::Message(body) = op.kind.as_ref() else {
            return Err("expected message".into());
        };
        assert!(body.attachments[0].inline_bytes().is_none());
        assert_eq!(
            body.attachments[0].staged().unwrap().size_bytes(),
            128 * 1024
        );

        queue
            .retry(id(50), RetryDecision::AcceptPossibleDuplicate)
            .await?;
        state(&session, id(50), OperationState::Sent).await?;
        assert!(queue.subscribe().await?.borrow().upload_progress.is_none());
        let received = f.server().take_received_upload().await.unwrap();
        assert_eq!(received.files[0].sha256, pattern_sha256(128 * 1024, 0x88));
        let _ = channel;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn progress_resets_when_not_dispatched_returns_to_queued() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let probe = PauseProbe::new(id(60), OutgoingDispatchPhase::Claimed);
    f.config.outgoing_probe = Some(Arc::clone(&probe) as Arc<dyn OutgoingDispatchProbe>);
    f.reopen()?;

    let (session, channel) = f.run(signed_in(&f))??;
    let mut rx = f.run(async {
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(128 * 1024, 0x99))
            .await?;
        let rx = queue.subscribe().await?;
        queue
            .enqueue(
                channel,
                id(60),
                OperationKind::Message(OutgoingMessage {
                    content: "not-dispatched".into(),
                    attachments: vec![OutgoingAttachment::from_staged("nd.bin", staged)],
                    ..Default::default()
                }),
                UnixMicros(60),
            )
            .await?;
        probe.wait_reached().await;
        let snap = rx.borrow().clone();
        let initial = snap
            .progress_for(id(60))
            .expect("Sending claim must publish initial 0-byte progress");
        assert_eq!(initial.bytes_uploaded, 0);
        assert_eq!(initial.total_bytes, 128 * 1024);
        Ok::<_, Box<dyn std::error::Error>>(rx)
    })??;

    // Stop accepting new TCP connections while keeping the gateway socket open so POST /messages
    // fails ECONNREFUSED (proven NotDispatched) and legitimately returns Sending -> Queued with
    // upload_progress reset to None.
    f.run(async {
        f.server().stop_accepting().await;
    })?;
    probe.release();

    f.run(async {
        loop {
            let snap = rx.borrow_and_update().clone();
            if let Some(op) = snap.operations.iter().find(|op| op.id == id(60))
                && op.state == OperationState::Queued
                && op.attempts == 1
            {
                assert!(
                    snap.upload_progress.is_none(),
                    "returning to Queued on proven NotDispatched must reset upload_progress"
                );
                break;
            }
            rx.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn concurrent_discard_during_dispatch_preparation_leaves_no_storage_fault() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let probe = PauseProbe::new(id(70), OutgoingDispatchPhase::Queued);
    f.config.outgoing_probe = Some(Arc::clone(&probe) as Arc<dyn OutgoingDispatchProbe>);
    f.reopen()?;

    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(64 * 1024, 0xaa))
            .await?;
        queue
            .enqueue(
                channel,
                id(70),
                OperationKind::Message(OutgoingMessage {
                    content: "discard-mid-prep".into(),
                    attachments: vec![OutgoingAttachment::from_staged("drop.bin", staged)],
                    ..Default::default()
                }),
                UnixMicros(70),
            )
            .await?;
        // Dispatcher has opened the attachment and paused right before claim_sending.
        probe.wait_reached().await;
        // Enqueue a follower in the same channel, then discard id(70) before releasing the probe.
        queue
            .enqueue(
                channel,
                id(71),
                OperationKind::Message(OutgoingMessage {
                    content: "after-discard".into(),
                    ..Default::default()
                }),
                UnixMicros(71),
            )
            .await?;
        queue.discard(id(70)).await?;
        let mut rx = queue.subscribe().await?;
        probe.release();

        // Every snapshot until id(71) reaches Sent must stay fault-free (no Unavailable/Storage blip).
        loop {
            let snap = rx.borrow_and_update().clone();
            assert!(
                snap.fault.is_none(),
                "discard racing dispatch preparation must never emit a queue fault: {:?}",
                snap.fault
            );
            assert!(snap.operations.iter().all(|op| op.id != id(70)));
            if snap
                .operations
                .iter()
                .any(|op| op.id == id(71) && op.state == OperationState::Sent)
            {
                break;
            }
            rx.changed().await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
