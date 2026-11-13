//! Canonical history isolation from damaged outgoing blobs and complete HTTP-only receipt projection.

use super::support::{Fixture, Result, channel, config, online, until};
use papo_sdk::{
    DataRoot, EmbedInput, EmbedMediaInput, OperationKind, OperationState, OutgoingAttachment,
    OutgoingFault, OutgoingMessage, UnixMicros, Uuid,
};
use papo_sdk_ui::{Timeline, TimelineItem};

fn local() -> Uuid {
    Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0010)
}
fn message() -> OutgoingMessage {
    OutgoingMessage {
        content: "synthetic".into(),
        attachments: vec![OutgoingAttachment {
            filename: "proof.bin".into(),
            bytes: vec![0, 255, 1],
        }],
        ..Default::default()
    }
}

#[test]
fn missing_or_corrupt_staged_files_do_not_block_live_or_focused_history() -> Result {
    for corrupt in [false, true] {
        let mut f = Fixture::new(
            10,
            vec![papo_test_server::RestStep {
                method: "POST".into(),
                path: "/messages".into(),
                delay_ms: 0,
                failure: Some(papo_test_server::Failure {
                    status: 500,
                    slug: "internal".into(),
                }),
            }],
        )?;
        let account = f.run(async {
            let session = f.signed_in("damaged").await?;
            let queue = session.outgoing();
            queue
                .enqueue(
                    channel(),
                    local(),
                    OperationKind::Message(message()),
                    UnixMicros(0),
                )
                .await?;
            let mut image = queue.subscribe().await?;
            loop {
                if image
                    .borrow_and_update()
                    .operations
                    .iter()
                    .any(|op| op.state == OperationState::Unknown)
                {
                    break;
                }
                image.changed().await?;
            }
            Ok::<_, Box<dyn std::error::Error>>(session.account())
        })??;
        f.close_client()?;
        let directory = DataRoot::new(f.profile.path()).outgoing_files(account);
        let file = std::fs::read_dir(directory)?.next().unwrap()?.path();
        if corrupt {
            std::fs::write(file, [1])?;
        } else {
            std::fs::remove_file(file)?;
        }
        f.reopen()?;
        f.run(async {
            let session = f.client().handle().session(account)?;
            online(&session).await?;
            let live = Timeline::live(session.clone(), channel(), config()).await?;
            let focused = Timeline::focused(
                session.clone(),
                channel(),
                f.messages[5].cursor().unwrap(),
                config(),
            )
            .await?;
            assert!(!live.subscribe()?.items.is_empty());
            assert!(!focused.subscribe()?.items.is_empty());
            let mut queue = session.outgoing().subscribe().await?;
            loop {
                if queue.borrow_and_update().fault == Some(OutgoingFault::Storage) {
                    break;
                }
                queue.changed().await?;
            }
            assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}

#[test]
fn attachment_backed_embed_is_complete_after_http_confirmation_without_gateway_delivery() -> Result
{
    let f = Fixture::new(10, vec![])?;
    f.run(async {
        let session=f.signed_in("receipt-embed").await?;
        f.server().set_silent_outgoing(true).await;
        let timeline=Timeline::live(session.clone(),channel(),config()).await?;
        let mut stream=timeline.subscribe()?;
        let mut body=message();
        body.embeds=vec![EmbedInput {title:Some("synthetic".into()),image:Some(EmbedMediaInput {attachment_name:Some("proof.bin".into()),..Default::default()}),..Default::default()}];
        session.outgoing().enqueue(channel(),local(),OperationKind::Message(body),UnixMicros(0)).await?;
        until(&mut stream,|s|s.items.iter().any(|item|matches!(item.as_ref(),TimelineItem::Message {message,..} if message.embeds.as_ref().is_some_and(|values|values.len()==1)))).await?;
        let projected=stream.items.iter().find_map(|item|match item.as_ref() {TimelineItem::Message {message,..} if message.embeds.as_ref().is_some_and(|v|!v.is_empty())=>Some(message),_=>None}).unwrap();
        let embed=&projected.embeds.as_ref().unwrap()[0];
        assert_eq!(embed.title.as_deref(),Some("synthetic"));
        assert_eq!(embed.image.as_ref().unwrap().size_bytes,Some(3));
        assert!(embed.image.as_ref().unwrap().url.as_ref().unwrap().starts_with("/media/"));
        assert_eq!(f.server().outgoing_request_counts().await.accepted,1);
        Ok::<_,Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn sent_receipt_stays_visible_during_cache_failure_then_retires_only_after_canonical_insertion()
-> Result {
    pending_receipt(false)
}

#[test]
fn canonical_delete_retires_a_confirmed_pending_receipt_without_resurrection() -> Result {
    pending_receipt(true)
}

fn pending_receipt(deleted: bool) -> Result {
    let f = Fixture::new(10, vec![])?;
    f.run(async {
        let session = f.signed_in("receipt-failure").await?;
        let generation = session.subscribe_connection().borrow().generation;
        f.server().set_silent_outgoing(true).await;
        let mut pins = f.server()
            .hold_next_rest("GET", &format!("/channels/{}/pinned", channel()))
            .await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        pins.wait_started().await?;
        let mut stream = timeline.subscribe()?;
        let history = session.history(channel()).await?;
        let db = rusqlite::Connection::open(
            DataRoot::new(f.profile.path()).account_cache(session.account()),
        )?;
        db.execute_batch(
            "CREATE TRIGGER fail_receipt BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'synthetic cache fault'); END;",
        )?;
        session.outgoing().enqueue(
            channel(), local(), OperationKind::Message(message()), UnixMicros(0),
        ).await?;
        let mut outgoing = session.outgoing().subscribe().await?;
        let remote = loop {
            let operation = outgoing.borrow_and_update().operations.iter()
                .find(|op| op.id == local() && op.state == OperationState::Sent)
                .cloned();
            if let Some(operation) = operation {
                assert!(!operation.history_projected);
                break operation.message_id.unwrap();
            }
            outgoing.changed().await?;
        };
        until(&mut stream, |snapshot| snapshot.items.iter().any(|item| {
            matches!(item.as_ref(), TimelineItem::LocalEcho { id, operation }
                if *id == local() && operation.state == OperationState::Sent
                    && operation.message_id == Some(remote))
        })).await?;
        assert!(!stream.items.iter().any(|item| {
            matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == remote)
        }));
        // Finish a pin response whose ticket predates the failed receipt's state reset.
        // The explicit read serializes behind that background response, proving it completed.
        pins.release();
        history.pins().await?;
        assert_eq!(session.subscribe_connection().borrow().generation, generation);
        let installed: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='fail_receipt'",
            [], |row| row.get(0),
        )?;
        assert_eq!(installed, 1);
        assert!(outgoing.borrow().operations.iter().any(|op| {
            op.id == local() && !op.history_projected
        }));
        let reads = f.server().read_request_counts().await;
        db.execute_batch("DROP TRIGGER fail_receipt;")?;
        drop(db);
        assert!(matches!(
            timeline.report_visible(stream.version, stream.generation, &[local()]).await,
            Err(papo_sdk_ui::TimelineError::StaleSnapshot)
        ));
        assert!(matches!(
            session.outgoing().retry(local(), papo_sdk::RetryDecision::AcceptPossibleDuplicate).await,
            Err(papo_sdk::OutgoingError::InvalidState)
        ));
        if deleted {
            f.server().delete_message(&remote.to_string()).await?;
            until(&mut stream, |snapshot| !snapshot.items.iter().any(|item| {
                matches!(item.as_ref(), TimelineItem::LocalEcho { id, .. } if *id == local())
            })).await?;
        } else {
            until(&mut stream, |snapshot| {
                let canonical = snapshot.items.iter().any(|item| {
                    matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == remote)
                });
                let echo = snapshot.items.iter().any(|item| {
                    matches!(item.as_ref(), TimelineItem::LocalEcho { id, .. } if *id == local())
                });
                canonical && !echo
            }).await?;
            assert_eq!(session.subscribe_connection().borrow().generation, generation);
            assert_eq!(f.server().read_request_counts().await.history_pages, reads.history_pages);
            loop {
                if outgoing.borrow_and_update().operations.iter().any(|op| {
                    op.id == local() && op.history_projected
                }) {
                    break;
                }
                outgoing.changed().await?;
            }
        }
        assert_eq!(f.server().outgoing_request_counts().await.accepted, 1);
        assert_eq!(f.server().outgoing_request_counts().await.requests, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
