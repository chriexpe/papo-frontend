//! End-to-end streamed attachment staging, 100 MiB throttled uploads, progress bounds and account fences.

use super::*;
use papo_sdk::{DataRoot, EmbedInput, EmbedMediaInput, UploadProgress};
use papo_test_server::UploadThrottle;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) const MIB: u64 = 1024 * 1024;
pub(super) const MAX_BYTES: u64 = 100 * MIB;

/// Deterministic zero-allocation streaming source with unknown/optional lying length.
pub(super) struct PatternReader {
    remaining: u64,
    offset: u64,
    seed: u8,
    fail_after: Option<u64>,
}

impl PatternReader {
    pub(super) fn new(len: u64, seed: u8) -> Self {
        Self {
            remaining: len,
            offset: 0,
            seed,
            fail_after: None,
        }
    }

    pub(super) fn failing(len: u64, seed: u8, fail_after: u64) -> Self {
        Self {
            remaining: len,
            offset: 0,
            seed,
            fail_after: Some(fail_after),
        }
    }
}

impl Read for PatternReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(limit) = self.fail_after
            && self.offset >= limit
        {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "synthetic mid-stream read failure",
            ));
        }
        if self.remaining == 0 {
            return Ok(0);
        }
        let mut want = usize::try_from(self.remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        if let Some(limit) = self.fail_after {
            let until_fail = usize::try_from(limit.saturating_sub(self.offset)).unwrap_or(want);
            want = want.min(until_fail.max(1));
        }
        for (i, slot) in buf[..want].iter_mut().enumerate() {
            let pos = self.offset + i as u64;
            *slot = ((pos.wrapping_mul(31) ^ u64::from(self.seed)) & 0xff) as u8;
        }
        self.offset += want as u64;
        self.remaining -= want as u64;
        Ok(want)
    }
}

pub(super) fn pattern_sha256(len: u64, seed: u8) -> [u8; 32] {
    let mut reader = PatternReader::new(len, seed);
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}

#[test]
fn throttled_100_mib_streamed_upload_reports_bounded_monotonic_progress_and_exact_sha256() -> Result
{
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();

        let staged = queue
            .stage_attachment(PatternReader::new(MAX_BYTES, 0x5a))
            .await?;
        assert_eq!(staged.account(), session.account());
        assert_eq!(staged.size_bytes(), MAX_BYTES);
        let expected_sha = pattern_sha256(MAX_BYTES, 0x5a);
        assert_eq!(staged.digest(), &expected_sha);

        // Throttle 100 MiB across ~10 pacing intervals (~500 ms total) so multiple coalesced ticks fire.
        f.server()
            .set_message_upload_throttle(Some(UploadThrottle {
                chunk_bytes: 10 * 1024 * 1024,
                delay: Duration::from_millis(50),
            }))
            .await;

        // Also enqueue an unrelated operation in another channel to verify its Arc stays untouched.
        let other_channel: Uuid = f
            .server()
            .create_channel("other-progress")
            .await?
            .id
            .unwrap()
            .parse()?;
        // Hold the other channel behind a queued-but-blocked op or just enqueue a second op in same channel.
        let mut rx = queue.subscribe().await?;
        let start = Instant::now();
        queue
            .enqueue(
                channel,
                id(10),
                OperationKind::Message(OutgoingMessage {
                    content: "100mib-proof".into(),
                    attachments: vec![OutgoingAttachment::from_staged("large-100mib.bin", staged)],
                    ..Default::default()
                }),
                UnixMicros(10),
            )
            .await?;
        queue
            .enqueue(
                other_channel,
                id(11),
                OperationKind::Message(OutgoingMessage {
                    content: "unrelated".into(),
                    ..Default::default()
                }),
                UnixMicros(11),
            )
            .await?;

        let (saw_initial_zero, saw_full_while_sending, progress_versions) =
            observe_100_mib_progress(&mut rx, session.account()).await?;
        let elapsed = start.elapsed();
        assert!(
            saw_initial_zero,
            "expected initial 0-byte progress at Sending"
        );
        assert!(
            saw_full_while_sending,
            "expected final 100% progress milestone before Sent"
        );
        // 100 MiB is 1,600 x 64 KiB chunks; coalesced publications must stay tightly bounded.
        assert!(
            (2..=64).contains(&progress_versions),
            "unexpected coalesced publication count {progress_versions} over {elapsed:?}"
        );
        println!(
            "PAPO_100MIB_UPLOAD publications={progress_versions} elapsed_ms={}",
            elapsed.as_millis()
        );

        let received = f
            .server()
            .take_received_upload()
            .await
            .ok_or("missing received upload")?;
        assert_eq!(
            received.fields.get("content").map(String::as_str),
            Some("100mib-proof")
        );
        assert_eq!(received.files.len(), 1);
        assert_eq!(received.files[0].filename, "large-100mib.bin");
        assert_eq!(received.files[0].size_bytes, MAX_BYTES);
        assert_eq!(received.files[0].sha256, expected_sha);

        // Wait for the unrelated second operation to complete (which guarantees id(10)'s
        // finish_attempt, including orphan collection, has released outgoing.serial).
        state(&session, id(11), OperationState::Sent).await?;
        let staging = DataRoot::new(f.profile.path()).outgoing_files(session.account());
        assert_eq!(std::fs::read_dir(staging)?.count(), 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

async fn observe_100_mib_progress(
    rx: &mut tokio::sync::watch::Receiver<Arc<papo_sdk::OutgoingSnapshot>>,
    account: papo_sdk::AccountId,
) -> Result<(bool, bool, usize)> {
    let mut progress_updates: Vec<UploadProgress> = Vec::new();
    let mut other_op_arc: Option<Arc<papo_sdk::Operation>> = None;
    let mut saw_initial_zero = false;
    let mut saw_full_while_sending = false;
    let mut progress_versions = 0_usize;

    loop {
        let snap = rx.borrow_and_update().clone();
        if let Some(other) = snap.operations.iter().find(|op| op.id == id(11)) {
            if let Some(prev) = &other_op_arc
                && prev.state == other.state
                && prev.history_projected == other.history_projected
            {
                assert!(
                    Arc::ptr_eq(prev, other),
                    "progress update must not replace unrelated operation Arc"
                );
            }
            if other.state == OperationState::Queued {
                other_op_arc = Some(Arc::clone(other));
            }
        }
        let op10 = snap.operations.iter().find(|op| op.id == id(10));
        if let Some(progress) = snap.progress_for(id(10)) {
            assert_eq!(progress.account, account);
            assert_eq!(progress.operation_id, id(10));
            assert_eq!(progress.total_bytes, MAX_BYTES);
            assert!(progress.bytes_uploaded <= MAX_BYTES);
            assert_eq!(op10.map(|op| op.state), Some(OperationState::Sending));
            if progress.bytes_uploaded == 0 {
                saw_initial_zero = true;
            }
            if progress.bytes_uploaded == MAX_BYTES {
                saw_full_while_sending = true;
            }
            if progress_updates.last() != Some(&progress) {
                if let Some(prev) = progress_updates.last() {
                    assert!(
                        progress.bytes_uploaded > prev.bytes_uploaded,
                        "progress must be strictly monotonic: {} -> {}",
                        prev.bytes_uploaded,
                        progress.bytes_uploaded
                    );
                }
                progress_updates.push(progress);
                progress_versions += 1;
            }
        }
        if op10.is_some_and(|op| op.state == OperationState::Sent) {
            assert!(
                snap.upload_progress.is_none(),
                "upload progress must clear on Sent"
            );
            return Ok((saw_initial_zero, saw_full_while_sending, progress_versions));
        }
        rx.changed().await?;
    }
}

#[test]
fn full_upload_transfer_is_not_sent_until_backend_confirms_and_multipart_preserves_order_and_embeds()
-> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        // Seed a message to reply to.
        let reply_id = Uuid::from_u128(0x7000_0000_0000_4000_8000_0000_0000_0099);
        f.server()
            .seed_messages(
                &channel.to_string(),
                vec![papo_sdk::Message {
                    id: Some(reply_id.to_string()),
                    channel_id: Some(channel.to_string()),
                    created_at: Some("2026-01-01T00:00:00.000000Z".into()),
                    ..Default::default()
                }],
            )
            .await?;

        let first = queue
            .stage_attachment(PatternReader::new(256 * 1024, 0x11))
            .await?;
        let second = queue
            .stage_attachment(PatternReader::new(128 * 1024, 0x22))
            .await?;
        let expected_first = pattern_sha256(256 * 1024, 0x11);
        let expected_second = pattern_sha256(128 * 1024, 0x22);

        // Hold AFTER the server receives 100% of the body, BEFORE it sends HTTP 201!
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        let mut rx = queue.subscribe().await?;
        queue
            .enqueue(
                channel,
                id(20),
                OperationKind::Message(OutgoingMessage {
                    content: "multi-order".into(),
                    reply_to: Some(MessageTarget::Remote(reply_id)),
                    embeds: vec![EmbedInput {
                        title: Some("attached".into()),
                        image: Some(EmbedMediaInput {
                            attachment_name: Some("first.png".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    attachments: vec![
                        OutgoingAttachment::from_staged("first.png", first),
                        OutgoingAttachment::from_staged("second.bin", second),
                    ],
                    ..Default::default()
                }),
                UnixMicros(20),
            )
            .await?;

        hold.wait_started().await?;
        // 100% of bytes reached the server, yet the operation MUST remain Sending, NOT Sent!
        loop {
            let snap = rx.borrow_and_update().clone();
            let op = snap.operations.iter().find(|op| op.id == id(20)).unwrap();
            assert_eq!(op.state, OperationState::Sending);
            if snap
                .progress_for(id(20))
                .is_some_and(|p| p.bytes_uploaded == 384 * 1024)
            {
                break;
            }
            rx.changed().await?;
        }

        hold.release();
        let sent = state(&session, id(20), OperationState::Sent).await?;
        assert!(rx.borrow_and_update().upload_progress.is_none());
        let received = f
            .server()
            .take_received_upload()
            .await
            .ok_or("missing received upload")?;
        assert_eq!(
            received.fields.get("reply_to").map(String::as_str),
            Some(reply_id.to_string().as_str())
        );
        assert_eq!(received.files.len(), 2);
        assert_eq!(received.files[0].filename, "first.png");
        assert_eq!(received.files[0].size_bytes, 256 * 1024);
        assert_eq!(received.files[0].sha256, expected_first);
        assert_eq!(received.files[1].filename, "second.bin");
        assert_eq!(received.files[1].size_bytes, 128 * 1024);
        assert_eq!(received.files[1].sha256, expected_second);
        let msg = sent.message.as_ref().unwrap();
        let embeds = msg.embeds.as_ref().unwrap();
        assert_eq!(embeds.len(), 1);
        assert_eq!(
            embeds[0].image.as_ref().unwrap().size_bytes,
            Some(256 * 1024)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
