//! Many unread channels, held REST and an explicit gateway burst measure bounded reconnect behavior.

use super::fixture::{self, Result};
use papo_sdk::{Client, ConnectionState};
use papo_test_server::{Config, TestServer};
use std::time::Instant;

pub async fn run(client: &Client, count: usize) -> Result {
    let server = TestServer::start(Config {
        history: (0..count).map(|index| fixture::seed(index, 2)).collect(),
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
    let previous = session.generation();
    let mut connection = session.subscribe_connection();
    server.set_gateway_accepting(false).await;
    server.disconnect_gateways();
    loop {
        if matches!(
            connection.borrow_and_update().state,
            ConnectionState::Backoff { .. }
        ) {
            break;
        }
        connection.changed().await?;
    }
    for index in 0..count {
        let mut burst = fixture::seed(index, 401);
        let first: papo_sdk::Uuid = burst.first_message_id.parse()?;
        burst.first_message_id = papo_sdk::Uuid::from_u128(first.as_u128() + 1000).to_string();
        burst.first_created_at = "2026-01-02T00:00:00.123456Z".into();
        server.seed_history(burst).await?;
    }
    let path = format!("/channels/{}/messages", fixture::seed(0, 1).channel_id);
    let mut hold = server.hold_next_rest("GET", &path).await?;
    let before = server.read_request_counts().await;
    let started = Instant::now();
    server.set_gateway_accepting(true).await;
    session.reconnect()?;
    hold.wait_started().await?;
    eprintln!("Reconnect fixture: history request held");
    for index in 0..900 {
        server
            .emit_event(serde_json::from_value(serde_json::json!({
                "type":"typing", "is_typing":true, "channel_id":fixture::seed(0, 1).channel_id,
                "user_id":papo_sdk::Uuid::from_u128(999).to_string(),
            }))?)
            .await?;
        if index % 32 == 31 {
            loop {
                let work = session.history_diagnostics().await?.work;
                assert_eq!(work.reconnect_overflows, 0);
                if work.reconnect_journal_peak > index {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    }
    loop {
        let work = session.history_diagnostics().await?.work;
        assert_eq!(work.reconnect_overflows, 0);
        if work.reconnect_journal_peak >= 900 {
            break;
        }
        tokio::task::yield_now().await;
    }
    eprintln!("Reconnect fixture: 900 gateway entries journaled");
    hold.release();
    fixture::online(&session, Some(previous)).await?;
    let after = server.read_request_counts().await;
    assert!(after.history_pages - before.history_pages <= count * 4);
    assert_eq!(after.uploads, 0);
    for index in 0..count {
        let state = session
            .reading_state(fixture::seed(index, 1).channel_id.parse()?)
            .ok_or("missing unread state")?;
        assert!(matches!(
            state.unread_count,
            papo_sdk::UnreadCount::AtLeast(_)
        ));
    }
    fixture::report(&session, "unread_reconnect_900_events", count, started).await?;
    Ok(())
}
