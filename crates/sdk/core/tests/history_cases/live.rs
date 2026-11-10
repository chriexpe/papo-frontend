//! Owns public live mutation precedence and payload replacement proofs; leaves reconnect and model internals outside.

use super::{AT, CHANNEL, FIRST, Fixture, Result, channel, config, signed_in, wait_event};
use papo_api::{
    HistoryQuery,
    gateway::{KnownEvent, MessageEditEvent},
    wire::{Embed, MessageReactionSummary},
};

#[test]
fn newer_live_edit_wins_and_delete_prevents_resurrection_by_earlier_edit() -> Result {
    let fixture = Fixture::new(config(40))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-edit").await?;
        let history = session.history(channel()).await?;
        let initial = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        let id = initial.messages[0].id.as_deref().unwrap();
        let mut events = session.subscribe_events();
        fixture
            .server()
            .edit_message(id, "", "2026-01-01T00:00:02.123456Z")
            .await?;
        wait_event(&mut events, "message_embeds_update").await?;
        let earlier = KnownEvent::MessageEdit(Box::new(MessageEditEvent {
            id: id.to_owned(),
            channel_id: CHANNEL.into(),
            content: String::new(),
            edited_at: "2026-01-01T00:00:01.123456Z".into(),
        }));
        fixture.server().emit_event(earlier.clone()).await?;
        wait_event(&mut events, "message_edit").await?;
        let newest = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        assert_eq!(
            newest.messages[0].edited_at.as_deref(),
            Some("2026-01-01T00:00:02.123456Z")
        );
        assert!(fixture.server().delete_message(id).await?);
        wait_event(&mut events, "message_delete").await?;
        fixture.server().emit_event(earlier).await?;
        wait_event(&mut events, "message_edit").await?;
        let after = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        assert!(
            after
                .messages
                .iter()
                .all(|message| message.id.as_deref() != Some(id))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_reaction_embed_and_detached_pin_snapshots_replace_without_chronological_pin_leak() -> Result
{
    let fixture = Fixture::new(config(40))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-payload").await?;
        let history = session.history(channel()).await?;
        let initial = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        let id = initial.messages[0].id.as_deref().unwrap();
        let mut events = session.subscribe_events();
        let unicode = char::from_u32(0x1f44d).ok_or("invalid scalar")?.to_string();
        fixture
            .server()
            .set_message_reaction(
                id,
                MessageReactionSummary {
                    unicode: Some(unicode.clone()),
                    count: Some(2),
                    ..MessageReactionSummary::default()
                },
            )
            .await?;
        wait_event(&mut events, "react_update").await?;
        fixture
            .server()
            .set_message_embeds(
                id,
                vec![Embed {
                    id: Some("90000000-0000-4000-8000-000000000001".into()),
                    source_type: Some("custom".into()),
                    fetch_method: Some("manual".into()),
                    created_at: Some(AT.into()),
                    ..Embed::default()
                }],
            )
            .await?;
        wait_event(&mut events, "message_embeds_update").await?;
        fixture.server().set_message_pin(FIRST, Some(AT)).await?;
        wait_event(&mut events, "message_pin").await?;
        let page = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        assert_eq!(
            page.messages[0].reactions.as_ref().unwrap()[0].count,
            Some(2)
        );
        assert_eq!(page.messages[0].embeds.as_ref().map(Vec::len), Some(1));
        assert!(
            page.messages
                .iter()
                .all(|message| message.id.as_deref() != Some(FIRST))
        );
        assert_eq!(history.pins().await?.value[0].id.as_deref(), Some(FIRST));
        fixture.server().set_message_pin(FIRST, None).await?;
        wait_event(&mut events, "message_pin").await?;
        assert!(history.pins().await?.value.is_empty());
        fixture
            .server()
            .set_message_reaction(
                id,
                MessageReactionSummary {
                    unicode: Some(unicode),
                    count: Some(0),
                    ..MessageReactionSummary::default()
                },
            )
            .await?;
        wait_event(&mut events, "react_update").await?;
        let page = history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        assert!(page.messages[0].reactions.as_ref().unwrap().is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
