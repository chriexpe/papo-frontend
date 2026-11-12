//! Framework-neutral local echo identity, closure and read exclusion; native composer integration is deferred.

use super::support::{Fixture, Result, channel, config, until};
use papo_sdk::{OperationKind, OperationState, OutgoingMessage, UnixMicros, Uuid};
use papo_sdk_ui::{Timeline, TimelineError, TimelineItem};
use papo_test_server::{Failure, RestStep};
use std::sync::Arc;

fn echo(items: &imbl::Vector<Arc<TimelineItem>>, id: Uuid) -> Option<&Arc<TimelineItem>> {
    items.iter().find(
        |item| matches!(item.as_ref(),TimelineItem::LocalEcho {id:current,..} if *current==id),
    )
}
#[test]
fn live_echoes_are_shared_stable_unread_excluded_and_replace_only_confirmed_server_ids() -> Result {
    let f = Fixture::new(10, vec![])?;
    f.run(async {
        let session = f.signed_in("echo").await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let another = Timeline::live(session.clone(), channel(), config()).await?;
        let mut stream = timeline.subscribe()?;
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        let local = Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0001);
        session
            .outgoing()
            .enqueue(
                channel(),
                local,
                OperationKind::Message(OutgoingMessage {
                    content: "synthetic".into(),
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        hold.wait_started().await?;
        until(&mut stream, |s| echo(&s.items, local).is_some()).await?;
        let old = Arc::clone(echo(&stream.items, local).unwrap());
        let second = another.subscribe()?;
        assert!(echo(&second.items, local).is_some());
        assert!(matches!(
            timeline
                .report_visible(stream.version, stream.generation, &[local])
                .await,
            Err(TimelineError::StaleSnapshot)
        ));
        let fresh = timeline.subscribe()?;
        assert!(Arc::ptr_eq(&old, echo(&fresh.items, local).unwrap()));
        hold.release();
        let mut outgoing = session.outgoing().subscribe().await?;
        let remote = loop {
            let remote = outgoing
                .borrow_and_update()
                .operations
                .iter()
                .find(|op| op.id == local && op.state == OperationState::Sent)
                .and_then(|op| op.message_id);
            if let Some(id) = remote {
                break id;
            }
            outgoing.changed().await?;
        };
        until(&mut stream, |s| {
            echo(&s.items, local).is_none()
                && s.items.iter().any(
                    |item| matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote),
                )
        })
        .await?;
        assert_eq!(
            stream
                .items
                .iter()
                .filter(|item| matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote))
                .count(),
            1
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn focused_windows_exclude_echoes_and_similar_remote_content_never_resolves_unknown() -> Result {
    let f = Fixture::new(
        10,
        vec![RestStep {
            method: "POST".into(),
            path: "/messages".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 500,
                slug: "internal".into(),
            }),
        }],
    )?;
    f.run(async {
        let session=f.signed_in("unknown-echo").await?;
        let live=Timeline::live(session.clone(),channel(),config()).await?;
        let focused=Timeline::focused(session.clone(),channel(),f.messages[5].cursor().unwrap(),config()).await?;
        let local=Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0001);
        session.outgoing().enqueue(channel(),local,OperationKind::Message(OutgoingMessage {content:"synthetic".into(),..Default::default()}),UnixMicros(0)).await?;
        let mut stream=live.subscribe()?;
        until(&mut stream,|s|echo(&s.items,local).is_some_and(|item|matches!(item.as_ref(),TimelineItem::LocalEcho {operation,..} if operation.state==OperationState::Unknown))).await?;
        assert!(echo(&focused.subscribe()?.items,local).is_none());
        let mut remote=f.messages[9].clone();remote.id=Some(Uuid::from_u128(0xe000_0000_0000_4000_8000_0000_0000_0001).to_string());remote.content=Some("synthetic".into());
        let remote_id=remote.id.as_ref().unwrap().parse::<Uuid>()?;
        f.server().publish_message(remote).await?;
        until(&mut stream,|s|s.items.iter().any(|item|matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote_id))).await?;
        let baseline=live.subscribe()?;
        assert!(echo(&baseline.items,local).is_some_and(|item|matches!(item.as_ref(),TimelineItem::LocalEcho {operation,..} if operation.state==OperationState::Unknown)));
        Ok::<_,Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn delete_received_before_http_receipt_wins_over_late_acknowledgement() -> Result {
    let f = Fixture::new(4, vec![])?;
    f.run(async {
        let session = f.signed_in("deleted-ack").await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let mut stream = timeline.subscribe()?;
        let local = Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0001);
        let remote = Uuid::from_u128(0x9000_0000_0000_4000_8000_0000_0000_0001);
        let mut hold = f.server().hold_next_rest("POST", "/messages").await?;
        session
            .outgoing()
            .enqueue(
                channel(),
                local,
                OperationKind::Message(OutgoingMessage {
                    content: "synthetic".into(),
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        hold.wait_started().await?;
        until(&mut stream, |s| {
            s.items
                .iter()
                .any(|item| matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote))
        })
        .await?;
        f.server().delete_message(&remote.to_string()).await?;
        until(&mut stream, |s| {
            !s.items
                .iter()
                .any(|item| matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote))
        })
        .await?;
        hold.release();
        let mut outgoing = session.outgoing().subscribe().await?;
        loop {
            if outgoing
                .borrow_and_update()
                .operations
                .iter()
                .any(|op| op.id == local && op.state == OperationState::Sent)
            {
                break;
            }
            outgoing.changed().await?;
        }
        let current = timeline.subscribe()?;
        assert!(
            !current
                .items
                .iter()
                .any(|item| matches!(item.as_ref(),TimelineItem::Message {id,..} if *id==remote))
        );
        assert!(echo(&current.items, local).is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
