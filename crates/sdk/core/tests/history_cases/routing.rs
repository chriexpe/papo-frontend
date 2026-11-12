//! Unscoped gateway events cannot mutate or fetch unrelated histories; held pages retain newer totals.

use super::{Fixture, Result, channel, config, recipe, signed_in, wait_event};
use papo_api::{
    HistoryQuery, Uuid,
    gateway::{KnownEvent, MessagePinEvent, ReactUpdateEvent},
};

#[test]
fn reaction_and_pin_leave_unrelated_channel_publication_and_pin_requests_untouched() -> Result {
    let mut scenario = config(4);
    let mut other = recipe(4);
    other.channel_id = Uuid::from_u128(900).to_string();
    other.first_message_id = Uuid::from_u128(1000).to_string();
    scenario.history.push(other.clone());
    let fixture = Fixture::new(scenario)?;
    fixture.run(async {
        let session = signed_in(&fixture, "routing").await?;
        let a = session.history(channel()).await?;
        let b = session.history(other.channel_id.parse()?).await?;
        let a_before = a.subscribe().await?.snapshot;
        let b_before = b.subscribe().await?.snapshot;
        let before = fixture.server().read_request_counts().await;
        let mut events = session.subscribe_events();
        let id = a_before.messages[0].id.clone().ok_or("missing id")?;
        fixture
            .server()
            .emit_event(KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
                message_id: id.clone(),
                unicode: Some("x".into()),
                count: 3,
                ..ReactUpdateEvent::default()
            })))
            .await?;
        wait_event(&mut events, "react_update").await?;
        fixture
            .server()
            .emit_event(KnownEvent::MessagePin(Box::new(MessagePinEvent {
                message_id: id,
                is_pinned: true,
            })))
            .await?;
        wait_event(&mut events, "message_pin").await?;
        fixture
            .server()
            .emit_event(KnownEvent::MessagePin(Box::new(MessagePinEvent {
                message_id: Uuid::from_u128(9999).to_string(),
                is_pinned: true,
            })))
            .await?;
        wait_event(&mut events, "message_pin").await?;
        let after = fixture.server().read_request_counts().await;
        let b_after = b.subscribe().await?.snapshot;
        assert!(std::sync::Arc::ptr_eq(&b_before, &b_after));
        assert_eq!(before.pin_lists, after.pin_lists);
        let a_after = a.subscribe().await?.snapshot;
        assert_eq!(
            a_after.messages[0].reactions.as_ref().unwrap()[0].count,
            Some(3)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn reaction_to_unknown_id_during_held_page_is_replayed_only_when_ownership_is_proved() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture, "routing-race").await?;
        fixture.server().seed_history(recipe(12)).await?;
        let history = session.history(channel()).await?;
        let mut events = session.subscribe_events();
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{}/messages", channel()))
            .await?;
        let pending = {
            let history = history.clone();
            tokio::spawn(async move {
                history
                    .page(HistoryQuery {
                        limit: Some(12),
                        ..HistoryQuery::default()
                    })
                    .await
            })
        };
        hold.wait_started().await?;
        let id = recipe(12).messages()?[0].id.clone().ok_or("missing id")?;
        fixture
            .server()
            .emit_event(KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
                message_id: id.clone(),
                unicode: Some("x".into()),
                count: 3,
                ..ReactUpdateEvent::default()
            })))
            .await?;
        wait_event(&mut events, "react_update").await?;
        hold.release();
        let page = pending.await??;
        let message = page
            .messages
            .iter()
            .find(|message| message.id.as_ref() == Some(&id))
            .ok_or("missing message")?;
        assert_eq!(message.reactions.as_ref().unwrap()[0].count, Some(3));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
