//! 30-row viewport visible reporting, active catch-up retraction, and arrival overlay cost at 10k/100k rows.

use super::fixture::{self, Result};
use papo_sdk::{Client, Message, Uuid, VisibilityContinuity};
use papo_sdk_ui::{Timeline, TimelineConfig, TimelineDirection, TimelineItem};
use papo_test_server::{Config, TestServer};
use std::time::Instant;

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
    let initial = fixture::hydrate(&history, count).await?;

    let config = TimelineConfig {
        page_size: 30,
        ..TimelineConfig::default()
    };
    let live = Timeline::live(session.clone(), channel, config.clone()).await?;
    let mid = initial.messages[count / 2]
        .cursor()
        .ok_or("missing mid cursor")?;
    let focused = Timeline::focused(session.clone(), channel, mid, config).await?;
    let old_anchor = initial.messages[0]
        .id
        .as_deref()
        .ok_or("missing oldest id")?
        .parse()?;
    session
        .reading_observer()
        .report_visible(
            channel,
            initial.generation,
            initial.version,
            &[old_anchor],
            VisibilityContinuity::Continuous,
        )
        .await?;

    let live_sub = live.subscribe()?;
    let head_30 = message_ids(&live_sub.items);
    assert_eq!(head_30.len(), 30);
    let started = Instant::now();
    live.report_visible(
        live_sub.version,
        live_sub.generation,
        &head_30,
        VisibilityContinuity::Discontinuous,
    )
    .await?;
    let state = live.reading_state()?;
    assert!(state.catch_up.is_some());
    fixture::report(&session, "skip_30_catch_up", count, started).await?;

    let _ = live.paginate(TimelineDirection::Older).await?;
    let scroll_sub = live.subscribe()?;
    let all_ids = message_ids(&scroll_sub.items);
    let mid_30 = &all_ids[all_ids.len().saturating_sub(35)..all_ids.len() - 5];
    assert_eq!(mid_30.len(), 30);
    let started = Instant::now();
    live.report_visible(
        scroll_sub.version,
        scroll_sub.generation,
        mid_30,
        VisibilityContinuity::Continuous,
    )
    .await?;
    let _ = live.reading_state()?;
    fixture::report(&session, "scroll_30_catch_up", count, started).await?;

    let mut events = session.subscribe_events();
    server
        .publish_message(Message {
            id: Some(Uuid::from_u128(900_000_001).to_string()),
            channel_id: Some(channel.to_string()),
            author_id: Some(Uuid::from_u128(999).to_string()),
            created_at: Some("2026-01-02T00:00:00.123456Z".into()),
            ..Message::default()
        })
        .await?;
    fixture::event(&mut events, "message").await?;
    let started = Instant::now();
    let overlay = focused.reading_state()?;
    assert_eq!(overlay.arrivals.count, 1);
    assert!(overlay.catch_up.is_some());
    fixture::report(&session, "arrival_overlay", count, started).await?;
    Ok(())
}

fn message_ids(items: &imbl::Vector<std::sync::Arc<TimelineItem>>) -> Vec<Uuid> {
    items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}
