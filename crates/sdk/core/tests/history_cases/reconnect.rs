//! Owns public reconnect/overlap and late-response proofs over loopback; leaves model algorithms outside.

use super::{
    AT, CHANNEL, FIRST, Fixture, Result, channel, config, disconnect, online, query, recipe,
    signed_in,
};
use papo_api::{HistoryOrder, HistoryQuery};
use papo_sdk::{ClientError, HistoryError, HistorySource};

#[test]
fn disconnected_creation_edit_delete_and_pin_are_revalidated_on_open_channel_reconnect() -> Result {
    let fixture = Fixture::new(config(45))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-offline").await?;
        let history = session.history(channel()).await?;
        history.page(query(HistoryOrder::Desc)).await?;
        let old = session.generation();
        disconnect(&fixture, &session).await?;
        let messages = recipe(48).messages()?;
        let edited = messages[43].id.as_deref().unwrap();
        let deleted = messages[42].id.as_deref().unwrap();
        fixture
            .server()
            .edit_message(edited, "", "2026-01-01T00:00:02.123456Z")
            .await?;
        assert!(fixture.server().delete_message(deleted).await?);
        fixture.server().set_message_pin(FIRST, Some(AT)).await?;
        for message in &messages[45..] {
            fixture.server().publish_message(message.clone()).await?;
        }
        let cached = history.page(query(HistoryOrder::Desc)).await?;
        assert_eq!(cached.source, HistorySource::Cache);
        assert!(cached.stale);
        assert_eq!(cached.messages.len(), 45);
        fixture.server().set_gateway_accepting(true).await;
        session.reconnect()?;
        let generation = online(&session).await?;
        assert!(generation.get() > old.get());
        let newest = history.page(query(HistoryOrder::Desc)).await?;
        assert_eq!(newest.generation, generation);
        assert!(!newest.stale);
        assert_eq!(newest.messages.len(), 47);
        assert!(
            newest
                .messages
                .iter()
                .all(|message| message.id.as_deref() != Some(deleted))
        );
        assert!(
            newest
                .messages
                .iter()
                .any(|message| message.id.as_deref() == Some(edited)
                    && message.edited_at.as_deref() == Some("2026-01-01T00:00:02.123456Z"))
        );
        assert_eq!(history.pins().await?.value[0].id.as_deref(), Some(FIRST));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn held_old_generation_history_response_is_rejected_and_cannot_overwrite_reconnect_payload()
-> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-stale").await?;
        fixture.server().seed_history(recipe(12)).await?;
        let history = session.history(channel()).await?;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{CHANNEL}/messages"))
            .await?;
        let pending = {
            let history = history.clone();
            tokio::spawn(async move { history.page(query(HistoryOrder::Desc)).await })
        };
        hold.wait_started().await?;
        let old = session.generation();
        fixture
            .server()
            .edit_message(FIRST, "", "2026-01-01T00:00:02.123456Z")
            .await?;
        session.reconnect()?;
        let generation = online(&session).await?;
        assert!(generation.get() > old.get());
        hold.release();
        assert!(matches!(
            pending.await?,
            Err(HistoryError::Session(ClientError::Superseded))
        ));
        let page = history.page(query(HistoryOrder::Desc)).await?;
        assert_eq!(page.generation, generation);
        assert!(
            page.messages
                .iter()
                .any(|message| message.id.as_deref() == Some(FIRST)
                    && message.edited_at.as_deref() == Some("2026-01-01T00:00:02.123456Z"))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn reconnect_budget_preserves_explicit_gap_offline_and_exact_cursor_fills_it_online() -> Result {
    let fixture = Fixture::new(config(20))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-gap").await?;
        let history = session.history(channel()).await?;
        let initial = history.page(query(HistoryOrder::Desc)).await?;
        assert_eq!(initial.messages.len(), 20);
        assert!(!initial.has_more);
        disconnect(&fixture, &session).await?;
        fixture
            .server()
            .seed_messages(
                CHANNEL,
                recipe(470).messages()?.into_iter().skip(20).collect(),
            )
            .await?;
        fixture.server().set_gateway_accepting(true).await;
        session.reconnect()?;
        let reconciled_generation = online(&session).await?;
        disconnect(&fixture, &session).await?;
        let disconnected_generation = session.generation();
        let mut cursor = None;
        let mut count = 0;
        let gap = loop {
            let page = history
                .page(HistoryQuery {
                    cursor: cursor.as_ref(),
                    ..query(HistoryOrder::Desc)
                })
                .await?;
            assert_eq!(page.source, HistorySource::Cache);
            count += page.messages.len();
            if let Some(gap) = page.gap {
                break gap;
            }
            assert!(page.has_more);
            let next = page.next_cursor.clone();
            assert!(next.is_some() && next != cursor);
            cursor = next;
        };
        assert_eq!(
            count,
            400,
            "reconciled generation {} disconnected generation {}",
            reconciled_generation.get(),
            disconnected_generation.get()
        );
        assert_eq!(gap.order, HistoryOrder::Desc);
        fixture.server().set_gateway_accepting(true).await;
        session.reconnect()?;
        online(&session).await?;
        let filled = history
            .page(HistoryQuery {
                cursor: Some(&gap.cursor),
                order: gap.order,
                limit: Some(100),
            })
            .await?;
        assert!(!filled.messages.is_empty());
        assert!(filled.gap.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
