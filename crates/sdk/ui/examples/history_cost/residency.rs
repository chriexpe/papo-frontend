//! Repeated multi-channel visits exercise bounded idle data and protected public windows.

use super::fixture::{self, Result};
use papo_sdk::{Client, HistoryQuery};
use papo_sdk_ui::{Timeline, TimelineConfig};
use papo_test_server::{Config, TestServer};
use std::time::Instant;

pub async fn run(client: &Client, count: usize) -> Result {
    let server = TestServer::start(Config {
        history: (0..count).map(|index| fixture::seed(index, 1)).collect(),
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
    let budget = papo_sdk::HistoryBudget::default();
    let active = Timeline::live(
        session.clone(),
        fixture::seed(0, 1).channel_id.parse()?,
        TimelineConfig::default(),
    )
    .await?;
    let baseline = active.subscribe()?;
    for _ in 0..3 {
        let started = Instant::now();
        for index in 1..count {
            let history = session
                .history(fixture::seed(index, 1).channel_id.parse()?)
                .await?;
            let raw = history.subscribe().await?.snapshot;
            assert_eq!(raw.messages.len(), 1);
            drop(history);
            let report = session.trim_history().await?;
            assert!(report.idle_channels <= budget.idle_channels);
            assert!(report.idle_bytes <= budget.idle_bytes);
        }
        let current = active.subscribe()?;
        assert_eq!(current.items.len(), baseline.items.len());
        fixture::report(&session, "channel_cycle", count, started).await?;
    }
    let history = session
        .history(fixture::seed(1, 1).channel_id.parse()?)
        .await?;
    let raw = history.subscribe().await?;
    drop(history);
    session.trim_history().await?;
    assert!(!raw.snapshot.messages.is_empty());
    drop(raw);
    drop((active, baseline));
    let report = session.trim_history().await?;
    assert_eq!(report.pinned_channels, 0);
    // Reopened cache data is stale until server verification; it never grants fresh coverage.
    let reopened = session
        .history(fixture::seed(1, 1).channel_id.parse()?)
        .await?;
    assert_eq!(
        reopened.page(HistoryQuery::default()).await?.messages.len(),
        1
    );
    Ok(())
}
