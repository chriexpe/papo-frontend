//! Account revocation, discarded dependency and receipt-residency integration gates; no native UI is involved.

use super::*;

fn message() -> OperationKind {
    OperationKind::Message(OutgoingMessage {
        content: "synthetic".into(),
        ..Default::default()
    })
}

#[test]
fn drafts_operations_and_dependencies_are_account_scoped_and_late_results_cannot_revive_removal()
-> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (a, channel) = signed_in(&f).await?;
        let auth = f.client().handle().auth(a.account().server())?;
        register(&auth, "outgoing-other").await?;
        let b = login(&auth, "outgoing-other").await?;
        fixture::online(&b).await?;
        let qa = a.outgoing();
        let qb = b.outgoing();
        qa.save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    attachments: vec![OutgoingAttachment {
                        filename: "private.bin".into(),
                        bytes: vec![0, 255, 1],
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            UnixMicros(0),
            None,
        )
        .await?;
        assert!(qb.draft(channel).await?.is_none());
        qb.save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    attachments: vec![OutgoingAttachment {
                        filename: "other.bin".into(),
                        bytes: vec![2],
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            UnixMicros(0),
            None,
        )
        .await?;
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        qa.enqueue(channel, id(1), message(), UnixMicros(0)).await?;
        hold.wait_started().await?;
        assert!(qb.subscribe().await?.borrow().operations.is_empty());
        assert!(matches!(
            qb.enqueue(
                channel,
                id(2),
                OperationKind::Delete {
                    target: MessageTarget::Local(id(1))
                },
                UnixMicros(0)
            )
            .await,
            Err(OutgoingError::NotFound)
        ));
        let receiver = qa.subscribe().await?;
        f.client().handle().remove_account(a.account()).await?;
        assert!(receiver.has_changed().is_err());
        assert!(
            !papo_sdk::DataRoot::new(f.profile.path())
                .outgoing_files(a.account())
                .exists()
        );
        hold.release();
        assert!(qa.draft(channel).await.is_err());
        assert_eq!(
            qb.draft(channel).await?.unwrap().body.message.attachments[0].bytes,
            [2]
        );
        qb.enqueue(channel, id(3), message(), UnixMicros(0)).await?;
        state(&b, id(3), OperationState::Sent).await?;
        assert_eq!(qb.subscribe().await?.borrow().operations.len(), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn discarding_unknown_fails_its_dependents_and_never_sends_them_without_the_parent() -> Result {
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
            .enqueue(channel, id(1), message(), UnixMicros(0))
            .await?;
        queue
            .enqueue(
                channel,
                id(2),
                OperationKind::Delete {
                    target: MessageTarget::Local(id(1)),
                },
                UnixMicros(0),
            )
            .await?;
        state(&session, id(1), OperationState::Unknown).await?;
        queue.discard(id(1)).await?;
        let child = state(&session, id(2), OperationState::Failed).await?;
        assert_eq!(child.failure, Some(papo_sdk::OperationFailure::Dependency));
        assert!(matches!(
            queue.retry(id(2), RetryDecision::DefiniteFailure).await,
            Err(OutgoingError::InvalidState)
        ));
        queue.discard(id(2)).await?;
        queue
            .enqueue(channel, id(3), message(), UnixMicros(0))
            .await?;
        state(&session, id(3), OperationState::Sent).await?;
        assert_eq!(f.server().outgoing_request_counts().await.requests, 2);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn confirmed_receipts_plateau_and_release_files_after_full_acknowledgement() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, channel) = signed_in(&f).await?;
        let queue = session.outgoing();
        queue
            .enqueue(
                channel,
                id(1),
                OperationKind::Message(OutgoingMessage {
                    attachments: vec![OutgoingAttachment {
                        filename: "proof.bin".into(),
                        bytes: vec![0, 255, 1],
                    }],
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        let first = state(&session, id(1), OperationState::Sent).await?;
        let OperationKind::Message(body) = first.kind.as_ref() else {
            return Err("creation changed kind".into());
        };
        assert_eq!(body.attachments[0].bytes.len(), 0);
        for n in 2..=270 {
            queue
                .enqueue(channel, id(n), message(), UnixMicros(0))
                .await?;
            state(&session, id(n), OperationState::Sent).await?;
        }
        let image = queue.subscribe().await?.borrow().clone();
        assert_eq!(image.operations.len(), 256);
        assert!(
            image
                .operations
                .iter()
                .all(|op| op.state == OperationState::Sent)
        );
        assert!(image.operations.iter().all(|op| op.id != id(1)));
        let staging = papo_sdk::DataRoot::new(f.profile.path()).outgoing_files(session.account());
        assert_eq!(std::fs::read_dir(staging)?.count(), 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn admission_backpressure_preserves_acknowledged_work_and_discard_releases_capacity() -> Result {
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
            .enqueue(channel, id(1), message(), UnixMicros(0))
            .await?;
        state(&session, id(1), OperationState::Unknown).await?;
        for n in 2..=256 {
            queue
                .enqueue(channel, id(n), message(), UnixMicros(0))
                .await?;
        }
        assert!(matches!(
            queue
                .enqueue(channel, id(257), message(), UnixMicros(0))
                .await,
            Err(OutgoingError::Budget)
        ));
        assert_eq!(queue.subscribe().await?.borrow().operations.len(), 256);
        queue.discard(id(256)).await?;
        queue
            .enqueue(channel, id(257), message(), UnixMicros(0))
            .await?;
        let image = queue.subscribe().await?.borrow().clone();
        assert_eq!(image.operations.len(), 256);
        assert!(
            image
                .operations
                .iter()
                .any(|op| op.id == id(1) && op.state == OperationState::Unknown)
        );
        assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
