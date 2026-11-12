//! Full-history single-mutation work budgets and independent small windows; no renderer is involved.

use super::fixture::{self, Result};
use papo_sdk::{Client, HistoryQuery, Message, Uuid};
use papo_sdk_ui::{Timeline, TimelineConfig, TimelineItem};
use papo_test_server::{Config, TestServer};
use std::{sync::Arc, time::Instant};

pub async fn run(client: &Client, count: usize) -> Result {
    if !(100..=100_000).contains(&count) {
        return Err("unsupported fixture row count".into());
    }
    let server = TestServer::start(Config {
        history: vec![fixture::seed(0, count)],
        ..Config::default()
    })
    .await?;
    let result = exercise(client, &server, count).await;
    let stopped = server.shutdown().await;
    result?;
    stopped?;
    Ok(())
}

async fn exercise(client: &Client, server: &TestServer, count: usize) -> Result {
    let session = fixture::sign_in(client, server).await?;
    let channel = fixture::seed(0, count).channel_id.parse()?;
    let history = session.history(channel).await?;
    let started = Instant::now();
    let mut cursor = None;
    loop {
        let page = history
            .page(HistoryQuery {
                cursor: cursor.as_ref(),
                limit: Some(100),
                ..HistoryQuery::default()
            })
            .await?;
        cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    let initial = history.subscribe().await?.snapshot;
    assert_eq!(initial.messages.len(), count);
    fixture::report(&session, "hydrate", count, started).await?;
    let config = TimelineConfig {
        page_size: 30,
        ..TimelineConfig::default()
    };
    let live = Timeline::live(session.clone(), channel, config.clone()).await?;
    let focused = Timeline::focused(
        session.clone(),
        channel,
        initial.messages[count / 2]
            .cursor()
            .ok_or("missing fixture cursor")?,
        config,
    )
    .await?;
    let live_before = live.subscribe()?.items;
    let focus_before = focused.subscribe()?.items;
    let id = initial.messages[count - 1]
        .id
        .as_deref()
        .ok_or("missing fixture id")?;
    let mut events = session.subscribe_events();
    let before = session.history_diagnostics().await?.work;
    let started = Instant::now();
    server
        .set_message_reaction(
            id,
            serde_json::from_value(serde_json::json!({"unicode":"x", "count":1}))?,
        )
        .await?;
    fixture::event(&mut events, "react_update").await?;
    let after = session.history_diagnostics().await?.work;
    assert_eq!(after.publication_entries - before.publication_entries, 1);
    assert_eq!(after.publication_encodes - before.publication_encodes, 1);
    assert!(after.persisted_rows - before.persisted_rows <= 3);
    let live_after = live.subscribe()?.items;
    let focus_after = focused.subscribe()?.items;
    assert_eq!(changed_messages(&live_before, &live_after), 1);
    assert_eq!(changed_messages(&focus_before, &focus_after), 0);
    assert!(Arc::ptr_eq(
        &initial.messages[0],
        &history.subscribe().await?.snapshot.messages[0]
    ));
    fixture::report(&session, "reaction", count, started).await?;

    mutations(&session, &history, server, count, &mut events, id).await
}

async fn mutations(
    session: &papo_sdk::Session,
    history: &papo_sdk::HistoryHandle,
    server: &TestServer,
    count: usize,
    events: &mut papo_sdk::SessionEvents,
    id: &str,
) -> Result {
    let channel = history.channel();
    let before = session.history_diagnostics().await?.work;
    let started = Instant::now();
    server
        .edit_message(id, "", "2026-01-02T00:00:00.123456Z")
        .await?;
    fixture::event(events, "message_embeds_update").await?;
    assert!(
        session
            .history_diagnostics()
            .await?
            .work
            .publication_entries
            - before.publication_entries
            <= 2
    );
    fixture::report(session, "edit", count, started).await?;

    let before = session.history_diagnostics().await?.work;
    let started = Instant::now();
    let arrival = Message {
        id: Some(Uuid::from_u128(900_000_000).to_string()),
        channel_id: Some(channel.to_string()),
        author_id: Some(Uuid::from_u128(999).to_string()),
        created_at: Some("2026-01-02T00:00:00.123456Z".into()),
        ..Message::default()
    };
    server.publish_message(arrival).await?;
    fixture::event(events, "message").await?;
    assert_eq!(
        session
            .history_diagnostics()
            .await?
            .work
            .publication_entries
            - before.publication_entries,
        1
    );
    fixture::report(session, "append", count, started).await?;

    let before = session.history_diagnostics().await?.work;
    let started = Instant::now();
    assert!(server.delete_message(id).await?);
    fixture::event(events, "message_delete").await?;
    assert_eq!(
        session
            .history_diagnostics()
            .await?
            .work
            .publication_entries
            - before.publication_entries,
        1
    );
    fixture::report(session, "delete", count, started).await?;

    let raw = history.subscribe().await?.snapshot;
    let visible: Vec<_> = raw
        .messages
        .iter()
        .rev()
        .take(5)
        .filter_map(|message| message.id.as_deref()?.parse::<Uuid>().ok())
        .collect();
    let started = Instant::now();
    session
        .report_visible(channel, raw.generation, raw.version, &visible)
        .await?;
    fixture::report(session, "visible", count, started).await?;
    let cursor = raw.messages[count / 2]
        .cursor()
        .ok_or("missing fixture cursor")?;
    let started = Instant::now();
    let page = history
        .page(HistoryQuery {
            cursor: Some(&cursor),
            limit: Some(100),
            ..HistoryQuery::default()
        })
        .await?;
    assert_eq!(page.messages.len(), 100);
    assert_eq!(page.source, papo_sdk::HistorySource::Cache);
    fixture::report(session, "cache_page_100", count, started).await?;
    Ok(())
}

fn changed_messages(
    previous: &imbl::Vector<Arc<TimelineItem>>,
    next: &imbl::Vector<Arc<TimelineItem>>,
) -> usize {
    previous.iter().filter(|old| {
        let TimelineItem::Message { id, .. } = old.as_ref() else { return false; };
        next.iter().find(|item| matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if current == id))
            .is_none_or(|new| !Arc::ptr_eq(old, new))
    }).count()
}
