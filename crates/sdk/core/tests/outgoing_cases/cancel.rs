//! Cancelling an attachment upload and the upload idle timeout over loopback.

use super::streaming::{MIB, PatternReader};
use super::*;
use papo_sdk::{OperationFailure, StagedAttachment};
use papo_test_server::UploadThrottle;
use std::time::Duration;

fn with_file(name: &str, staged: StagedAttachment) -> OperationKind {
    OperationKind::Message(OutgoingMessage {
        content: name.into(),
        attachments: vec![OutgoingAttachment::from_staged(name, staged)],
        ..Default::default()
    })
}

#[test]
fn a_queued_upload_cancels_into_failed_and_retry_sends_it() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let first = queue.stage_attachment(PatternReader::new(MIB, 1)).await?;
        let second = queue.stage_attachment(PatternReader::new(MIB, 2)).await?;
        let mut hold = f
            .server()
            .hold_message_upload_after_bytes(64 * 1024)
            .await?;
        queue
            .enqueue(channel, id(1), with_file("first.bin", first), UnixMicros(0))
            .await?;
        hold.wait_started().await?;
        // Same channel: the second waits behind the first, still Queued.
        queue
            .enqueue(
                channel,
                id(2),
                with_file("second.bin", second),
                UnixMicros(0),
            )
            .await?;
        queue.cancel_upload(id(2)).await?;
        let failed = state(&session, id(2), OperationState::Failed).await?;
        assert_eq!(failed.failure, Some(OperationFailure::Cancelled));
        hold.release();
        state(&session, id(1), OperationState::Sent).await?;
        queue.retry(id(2), RetryDecision::DefiniteFailure).await?;
        state(&session, id(2), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn an_upload_cancelled_mid_body_is_failed_not_unknown() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        // Larger than loopback socket buffers, so the hold really stops the body.
        let staged = queue
            .stage_attachment(PatternReader::new(48 * MIB, 3))
            .await?;
        let mut hold = f
            .server()
            .hold_message_upload_after_bytes(256 * 1024)
            .await?;
        queue
            .enqueue(channel, id(1), with_file("big.bin", staged), UnixMicros(0))
            .await?;
        hold.wait_started().await?;
        state(&session, id(1), OperationState::Sending).await?;
        queue.cancel_upload(id(1)).await?;
        let failed = state(&session, id(1), OperationState::Failed).await?;
        assert_eq!(failed.failure, Some(OperationFailure::Cancelled));
        assert!(queue.subscribe().await?.borrow().upload_progress.is_none());
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_cancel_after_the_whole_body_left_stays_unknown() -> Result {
    let f = Fixture::new(Config {
        rest: vec![RestStep {
            method: "POST".into(),
            path: "/messages".into(),
            delay_ms: 3_000,
            failure: None,
        }],
        ..Default::default()
    })?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(64 * 1024, 4))
            .await?;
        let mut rx = queue.subscribe().await?;
        queue
            .enqueue(
                channel,
                id(1),
                with_file("small.bin", staged),
                UnixMicros(0),
            )
            .await?;
        loop {
            let done = rx
                .borrow_and_update()
                .progress_for(id(1))
                .is_some_and(|progress| progress.bytes_uploaded == progress.total_bytes);
            if done {
                break;
            }
            rx.changed().await?;
        }
        queue.cancel_upload(id(1)).await?;
        let unknown = state(&session, id(1), OperationState::Unknown).await?;
        assert_eq!(unknown.failure, None);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_message_without_files_has_no_upload_to_cancel() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        queue
            .enqueue(
                channel,
                id(1),
                OperationKind::Message(OutgoingMessage {
                    content: "text only".into(),
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        assert!(matches!(
            queue.cancel_upload(id(1)).await,
            Err(OutgoingError::InvalidState)
        ));
        assert!(matches!(
            queue.cancel_upload(id(9)).await,
            Err(OutgoingError::NotFound)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_slow_upload_outlives_the_request_timeout_while_bytes_flow() -> Result {
    let f = Fixture::with_request_timeout(Config::default(), Duration::from_millis(800))?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        f.server()
            .set_message_upload_throttle(Some(UploadThrottle {
                chunk_bytes: 256 * 1024,
                delay: Duration::from_millis(100),
            }))
            .await;
        let queue = session.outgoing();
        // About 3 s at the throttled rate, well past the 0.8 s request timeout.
        let staged = queue
            .stage_attachment(PatternReader::new(8 * MIB, 5))
            .await?;
        queue
            .enqueue(channel, id(1), with_file("slow.bin", staged), UnixMicros(0))
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_stalled_upload_times_out_into_unknown() -> Result {
    let f = Fixture::with_request_timeout(Config::default(), Duration::from_millis(800))?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let staged = queue
            .stage_attachment(PatternReader::new(48 * MIB, 6))
            .await?;
        let mut hold = f
            .server()
            .hold_message_upload_after_bytes(256 * 1024)
            .await?;
        queue
            .enqueue(
                channel,
                id(1),
                with_file("stalled.bin", staged),
                UnixMicros(0),
            )
            .await?;
        hold.wait_started().await?;
        state(&session, id(1), OperationState::Unknown).await?;
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
