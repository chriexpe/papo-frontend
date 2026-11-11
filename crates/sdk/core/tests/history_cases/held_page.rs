//! Owns public concurrent gateway/REST merge proofs; leaves pure overlay and persistence mechanics outside.

use super::{
    CHANNEL, FIRST, Fixture, Result, channel, config, query, recipe, signed_in, wait_event,
};
use papo_api::{HistoryOrder, HistoryQuery};
use papo_sdk::HistorySource;

#[test]
fn held_server_page_keeps_newer_live_edits_and_deletes_and_exact_far_boundary_cursor() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-overlay").await?;
        fixture.server().seed_history(recipe(12)).await?;
        let history = session.history(channel()).await?;
        let mut events = session.subscribe_events();
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{CHANNEL}/messages"))
            .await?;
        let generation = session.generation();
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
        let messages = recipe(12).messages()?;
        let edited = messages[1].id.as_deref().unwrap();
        fixture
            .server()
            .edit_message(edited, "", "2026-01-01T00:00:02.123456Z")
            .await?;
        wait_event(&mut events, "message_embeds_update").await?;
        assert!(fixture.server().delete_message(FIRST).await?);
        wait_event(&mut events, "message_delete").await?;
        hold.release();
        let page = pending.await??;
        assert_eq!(page.generation, generation);
        assert_eq!(page.source, HistorySource::Server);
        assert_eq!(page.messages.len(), 11);
        assert!(!page.has_more);
        assert!(
            page.messages
                .iter()
                .all(|message| message.id.as_deref() != Some(FIRST))
        );
        assert!(
            page.messages
                .iter()
                .any(|message| message.id.as_deref() == Some(edited)
                    && message.edited_at.as_deref() == Some("2026-01-01T00:00:02.123456Z"))
        );
        let boundary = page.next_cursor.ok_or("missing far boundary cursor")?;
        assert_eq!(boundary.id, FIRST);
        assert_eq!(boundary.created_at, super::AT);
        let cached = history.page(query(HistoryOrder::Desc)).await?;
        assert!(
            cached
                .messages
                .iter()
                .all(|message| message.id.as_deref() != Some(FIRST))
        );
        assert_eq!(cached.messages.len(), 11);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
