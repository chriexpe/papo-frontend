//! Public durable outgoing/draft behavior over loopback; process-kill gates are a separate Linux suite.

#![cfg(feature = "test-support")]
#[path = "outgoing_cases/expressions.rs"]
mod expressions;
mod fixture;
#[path = "outgoing_cases/lifecycle.rs"]
mod lifecycle;
#[path = "outgoing_cases/staging.rs"]
mod staging;
use fixture::{Fixture, Result, login, register};
use papo_sdk::{
    DraftBody, MessageTarget, OperationKind, OperationState, OutgoingAttachment, OutgoingError,
    OutgoingMessage, RetryDecision, Session, UnixMicros, Uuid,
};
use papo_test_server::{Config, Failure, RestStep};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xb000_0000_0000_4000_8000_0000_0000_0000 + n)
}
async fn signed_in(f: &Fixture) -> Result<(Session, Uuid)> {
    let handle = f.client().handle();
    let server = handle
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    register(&auth, "outgoing").await?;
    let channel = f
        .server()
        .create_channel("outgoing")
        .await?
        .id
        .unwrap()
        .parse()?;
    let session = login(&auth, "outgoing").await?;
    fixture::online(&session).await?;
    Ok((session, channel))
}
async fn state(
    session: &Session,
    local: Uuid,
    wanted: OperationState,
) -> Result<std::sync::Arc<papo_sdk::Operation>> {
    let mut rx = session.outgoing().subscribe().await?;
    loop {
        if let Some(op) = rx
            .borrow_and_update()
            .operations
            .iter()
            .find(|op| op.id == local && op.state == wanted)
            .cloned()
        {
            return Ok(op);
        }
        rx.changed().await?;
    }
}

#[test]
fn full_drafts_keep_immutable_files_newer_edits_and_explicit_local_dependencies() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let (account, channel) = f.run(signed_in(&f))??;
    f.run(async {
        let queue = account.outgoing();
        let mut body = DraftBody {
            message: OutgoingMessage {
                attachments: vec![OutgoingAttachment {
                    filename: "original.png".into(),
                    bytes: vec![0, 255, 1],
                }],
                ..Default::default()
            },
            mentions: vec![account.account().user()],
        };
        let revision = queue
            .save_draft(channel, body.clone(), UnixMicros(5), Some(0))
            .await?;
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        queue
            .enqueue_draft(channel, id(1), revision, UnixMicros(50))
            .await?;
        hold.wait_started().await?;
        body.message.attachments[0].filename = "newer.png".into();
        body.message.attachments[0].bytes = vec![2];
        let newer = queue
            .save_draft(channel, body, UnixMicros(6), Some(revision))
            .await?;
        enqueue_dependents(&queue, channel).await?;
        hold.release();
        let first = state(&account, id(1), OperationState::Sent).await?;
        let second = state(&account, id(2), OperationState::Sent).await?;
        state(&account, id(5), OperationState::Sent).await?;
        assert_eq!(
            second.message.as_ref().unwrap().reply_to.as_deref(),
            first.message.as_ref().unwrap().id.as_deref()
        );
        assert_eq!(
            first
                .message
                .as_ref()
                .unwrap()
                .attachments
                .as_ref()
                .unwrap()[0]
                .original_file_name
                .as_deref(),
            Some("original.png")
        );
        assert_eq!(queue.draft(channel).await?.unwrap().revision, newer);
        assert_eq!(
            queue
                .draft(channel)
                .await?
                .unwrap()
                .body
                .message
                .attachments[0]
                .bytes,
            [2]
        );
        assert!(matches!(
            queue
                .enqueue(
                    channel,
                    id(1),
                    OperationKind::Message(payload()),
                    UnixMicros(0)
                )
                .await,
            Err(OutgoingError::AlreadyExists)
        ));
        assert!(matches!(
            queue
                .enqueue_draft(channel, id(9), revision, UnixMicros(0))
                .await,
            Err(OutgoingError::StaleDraft)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    let account_id = account.account();
    drop(account);
    f.reopen()?;
    f.run(async {
        let restored = f.client().handle().session(account_id)?;
        assert_eq!(
            restored
                .outgoing()
                .draft(channel)
                .await?
                .unwrap()
                .body
                .message
                .attachments[0]
                .bytes,
            [2]
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn definite_rejection_blocks_channel_until_explicit_resolution_and_sent_clears_captured_draft()
-> Result {
    let f = Fixture::new(Config {
        rest: vec![RestStep {
            method: "POST".into(),
            path: "/messages".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 403,
                slug: "forbidden".into(),
            }),
        }],
        ..Default::default()
    })?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        let revision = queue
            .save_draft(
                channel,
                DraftBody {
                    message: payload(),
                    ..Default::default()
                },
                UnixMicros(0),
                None,
            )
            .await?;
        queue
            .enqueue_draft(channel, id(1), revision, UnixMicros(2))
            .await?;
        queue
            .enqueue(
                channel,
                id(2),
                OperationKind::Message(payload()),
                UnixMicros(1),
            )
            .await?;
        state(&session, id(1), OperationState::Failed).await?;
        assert_eq!(queue.draft(channel).await?.unwrap().revision, revision);
        assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
        queue.retry(id(1), RetryDecision::DefiniteFailure).await?;
        state(&session, id(2), OperationState::Sent).await?;
        assert!(queue.draft(channel).await?.unwrap().revision > revision);
        assert_eq!(f.server().outgoing_request_counts().await.accepted, 2);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn ambiguous_5xx_never_retries_on_reconnect_and_requires_duplicate_acknowledgement() -> Result {
    let f = Fixture::new(Config {
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
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        queue
            .enqueue(
                channel,
                id(1),
                OperationKind::Message(payload()),
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Unknown).await?;
        assert!(matches!(
            queue.retry(id(1), RetryDecision::DefiniteFailure).await,
            Err(OutgoingError::DuplicateRisk)
        ));
        session.reconnect()?;
        fixture::online(&session).await?;
        assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
        queue
            .retry(id(1), RetryDecision::AcceptPossibleDuplicate)
            .await?;
        state(&session, id(1), OperationState::Sent).await?;
        assert_eq!(f.server().outgoing_request_counts().await.requests, 2);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

async fn enqueue_dependents(queue: &papo_sdk::OutgoingService, channel: Uuid) -> Result {
    queue
        .enqueue(
            channel,
            id(2),
            OperationKind::Message(OutgoingMessage {
                content: "synthetic".into(),
                reply_to: Some(MessageTarget::Local(id(1))),
                ..Default::default()
            }),
            UnixMicros(1),
        )
        .await?;
    queue
        .enqueue(
            channel,
            id(3),
            OperationKind::Edit {
                target: MessageTarget::Local(id(2)),
                content: "synthetic-edit".into(),
                embeds: None,
            },
            UnixMicros(0),
        )
        .await?;
    queue
        .enqueue(
            channel,
            id(4),
            OperationKind::Reaction {
                target: MessageTarget::Local(id(2)),
                emoji_id: None,
                unicode: Some("👍".into()),
                add: true,
            },
            UnixMicros(0),
        )
        .await?;
    queue
        .enqueue(
            channel,
            id(5),
            OperationKind::Delete {
                target: MessageTarget::Local(id(2)),
            },
            UnixMicros(-1),
        )
        .await?;
    Ok(())
}

fn payload() -> OutgoingMessage {
    OutgoingMessage {
        content: "synthetic".into(),
        ..Default::default()
    }
}
