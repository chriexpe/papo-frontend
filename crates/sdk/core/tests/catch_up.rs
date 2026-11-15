//! Behaviour and durability specs for Session 13b catch-up intervals and saved reading positions.

#![cfg(feature = "test-support")]

mod fixture;
#[path = "catch_up_cases/intervals.rs"]
mod intervals;
#[path = "catch_up_cases/positions.rs"]
mod positions;
#[path = "catch_up_cases/reconcile_and_accounts.rs"]
mod reconcile_and_accounts;
#[path = "catch_up_cases/sparse_visibility.rs"]
mod sparse_visibility;

use fixture::{Fixture, Result, login, register};
use papo_api::{HistoryQuery, Uuid, wire::Message};
use papo_sdk::{
    CatchUpInterval, ConnectionState, HistoryError, HistorySnapshot, ReadPoint, ReadState,
    ReadingError, Session, UnixMicros, VisibilityContinuity,
};
use papo_test_server::{Config, HistorySeed};
use std::sync::Arc;
use tokio::sync::watch;

pub(crate) const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
pub(crate) const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";
pub(crate) const FIRST: &str = "70000000-0000-4000-8000-000000000001";

pub(crate) fn channel() -> Uuid {
    CHANNEL.parse().unwrap()
}

pub(crate) fn config(count: usize) -> Config {
    Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: AUTHOR.into(),
            first_message_id: FIRST.into(),
            count,
            first_created_at: "2026-01-01T00:00:00.123450Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    }
}

pub(crate) async fn signed_in_as(fixture: &Fixture, username: &str) -> Result<Session> {
    let handle = fixture.client().handle();
    let base_url = fixture.server().base_url();
    let existing = handle
        .subscribe_registry()
        .borrow()
        .servers
        .iter()
        .find(|s| s.address.as_str() == base_url)
        .map(|s| s.address.id());
    let server = match existing {
        Some(id) => id,
        None => handle.add_server(&base_url, UnixMicros(0)).await?,
    };
    let auth = handle.auth(server)?;
    register(&auth, username).await?;
    let session = login(&auth, username).await?;
    fixture::online(&session).await?;
    Ok(session)
}

pub(crate) async fn signed_in(fixture: &Fixture) -> Result<Session> {
    signed_in_as(fixture, "catchup").await
}

pub(crate) async fn load_history(session: &Session, ch: Uuid) -> Result<Arc<HistorySnapshot>> {
    let history = session.history(ch).await?;
    history
        .page(HistoryQuery {
            limit: Some(100),
            ..HistoryQuery::default()
        })
        .await?;
    Ok(history.subscribe().await?.snapshot)
}

pub(crate) fn msg_id(snapshot: &HistorySnapshot, index: usize) -> Uuid {
    snapshot.messages[index]
        .id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap()
}

pub(crate) fn msg_point(snapshot: &HistorySnapshot, index: usize) -> ReadPoint {
    let msg = &snapshot.messages[index];
    let at = chrono::DateTime::parse_from_rfc3339(msg.created_at.as_deref().unwrap()).unwrap();
    ReadPoint {
        created_at_us: at.timestamp_micros(),
        message_id: msg.id.as_deref().unwrap().parse().unwrap(),
    }
}

pub(crate) fn synthetic_message(
    channel_id: &str,
    author_id: &str,
    seq: u128,
    created_at: &str,
) -> Message {
    let mut msg = HistorySeed {
        channel_id: channel_id.into(),
        author_id: author_id.into(),
        first_message_id: Uuid::from_u128(0x7000_0000_0000_4000_8000_0000_0000_0000 + seq)
            .to_string(),
        count: 1,
        first_created_at: created_at.into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 1,
    }
    .messages()
    .unwrap()
    .pop()
    .unwrap();
    msg.author_id = Some(author_id.into());
    msg
}

pub(crate) async fn report_range(
    session: &Session,
    ch: Uuid,
    indices: std::ops::RangeInclusive<usize>,
    continuity: VisibilityContinuity,
) -> Result<Arc<HistorySnapshot>> {
    report_with(
        &session.reading_observer(),
        session,
        ch,
        indices,
        continuity,
    )
    .await
}

pub(crate) async fn report_with(
    observer: &papo_sdk::ReadingObserver,
    session: &Session,
    ch: Uuid,
    indices: std::ops::RangeInclusive<usize>,
    continuity: VisibilityContinuity,
) -> Result<Arc<HistorySnapshot>> {
    let history = session.history(ch).await?;
    for _ in 0..16 {
        let current = history.subscribe().await?.snapshot;
        let ids: Vec<Uuid> = indices.clone().map(|i| msg_id(&current, i)).collect();
        match observer
            .report_visible(ch, current.generation, current.version, &ids, continuity)
            .await
        {
            Ok(()) => return Ok(current),
            Err(ReadingError::History(HistoryError::StaleSnapshot)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err("canonical history did not stabilize".into())
}

pub(crate) fn interval(session: &Session, ch: Uuid) -> Option<CatchUpInterval> {
    session.catch_up_state(ch).and_then(|state| state.interval)
}

pub(crate) async fn wait_online(session: &Session) -> Result {
    let mut status = session.subscribe_connection();
    fixture::wait_for(&mut status, |s| s.state == ConnectionState::Online).await?;
    Ok(())
}

pub(crate) async fn wait_read(
    rx: &mut watch::Receiver<Arc<papo_sdk::ReadingSnapshot>>,
    ch: Uuid,
    predicate: impl Fn(&ReadState) -> bool,
) -> Result<ReadState> {
    loop {
        if let Some(state) = rx
            .borrow_and_update()
            .channels
            .get(&ch)
            .filter(|s| predicate(s))
            .cloned()
        {
            return Ok(state);
        }
        rx.changed().await?;
    }
}
