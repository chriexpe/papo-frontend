//! Synthetic loopback fixture admission and count-only reporting; SDK policy remains in production services.

use papo_sdk::{
    Client, ConnectionState, LoginRequest, RegisterRequest, Session, SessionEvents, UnixMicros,
    Uuid,
};
use papo_test_server::{HistorySeed, TestServer};
use std::{error::Error, time::Instant};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

pub fn seed(index: usize, count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: Uuid::from_u128(10_000 + index as u128).to_string(),
        author_id: Uuid::from_u128(999).to_string(),
        first_message_id: Uuid::from_u128(1_000_000 + (index as u128) * 1_000_000).to_string(),
        count,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

pub async fn sign_in(client: &Client, server: &TestServer) -> Result<Session> {
    let handle = client.handle();
    let saved = handle.add_server(&server.base_url(), UnixMicros(0)).await?;
    let auth = handle.auth(saved)?;
    let password: String = (0_u8..32)
        .map(|n| char::from(b'A' + n % 26))
        .chain(['!', '1'])
        .collect();
    auth.register(RegisterRequest {
        username: "costfixture".into(),
        password: password.clone(),
    })
    .await?;
    let session = auth
        .login(
            LoginRequest {
                username: "costfixture".into(),
                password,
            },
            UnixMicros(0),
        )
        .await?;
    online(&session, None).await?;
    Ok(session)
}

pub async fn online(session: &Session, after: Option<papo_sdk::Generation>) -> Result {
    let mut status = session.subscribe_connection();
    loop {
        let current = *status.borrow_and_update();
        if current.state == ConnectionState::Online
            && after.is_none_or(|after| current.generation > after)
        {
            return Ok(());
        }
        status.changed().await?;
    }
}

pub async fn event(events: &mut SessionEvents, expected: &str) -> Result {
    loop {
        if events.next().await?.value.kind() == expected {
            return Ok(());
        }
    }
}

pub async fn report(session: &Session, scenario: &str, rows: usize, started: Instant) -> Result {
    let diagnostics = session.history_diagnostics().await?;
    let elapsed_us_advisory = started.elapsed().as_micros();
    let process_peak_rss_bytes = tokio::task::spawn_blocking(peak_rss_bytes).await?;
    let record = serde_json::json!({
        "schema_version": 1, "scenario": scenario, "rows": rows,
        "elapsed_us_advisory": elapsed_us_advisory,
        "diagnostics": diagnostics,
        "process_peak_rss_bytes": process_peak_rss_bytes,
        "process_rss_scope": "SDK plus in-process synthetic backend and fixture data",
        "not_measured": ["allocations", "GPU", "native rendering"],
    });
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}
