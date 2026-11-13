//! Session 7c fixture conventions applied to physical allocation and bounded history reclamation.

use super::fixture::{self, Result};
use papo_sdk::{Client, HistoryQuery};
use papo_test_server::{Config, TestServer};
use std::time::Instant;

pub async fn run(client: &Client, count: usize) -> Result {
    if !(10_000..=100_000).contains(&count) {
        return Err("unsupported disk fixture row count".into());
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
    let history = session
        .history(fixture::seed(0, count).channel_id.parse()?)
        .await?;
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
    let before = history.subscribe().await?.snapshot;
    assert_eq!(before.messages.len(), count);
    let started = Instant::now();
    let report = client.handle().trim_disk().await?;
    assert!(!report.pressure);
    assert!(report.history_turns > 0);
    assert!(report.max_history_rows_per_turn <= 80);
    let after = history.subscribe().await?.snapshot;
    assert!(after.messages.len() < count);
    assert!(!after.gaps.is_empty());
    assert_eq!(after.tombstones.len(), 0);
    println!(
        "{}",
        serde_json::to_string(
            &serde_json::json!({"schema_version":1,"scenario":"disk","rows":count,"physical":report,"retained_messages":after.messages.len(),"explicit_gaps":after.gaps.len(),"elapsed_us_advisory":started.elapsed().as_micros()})
        )?
    );
    fixture::report(&session, "disk-residency", count, started).await
}
