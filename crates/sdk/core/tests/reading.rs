//! Public ordered reading/upload lifecycle over loopback TCP; deterministic storage/count policy has unit tests.

#![cfg(feature = "test-support")]

mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_api::{HistoryQuery, Uuid};
use papo_sdk::{ConnectionState, HistorySnapshot, ReadState, ReadingSnapshot, Session, UnixMicros};
use papo_test_server::{Config, Failure, HistorySeed, RestStep};
use std::sync::Arc;
use tokio::sync::watch;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const FIRST: &str = "70000000-0000-4000-8000-000000000001";
fn channel() -> Uuid {
    CHANNEL.parse().unwrap()
}
fn config() -> Config {
    Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: "20000000-0000-4000-8000-000000000001".into(),
            first_message_id: FIRST.into(),
            count: 10,
            first_created_at: "2026-01-01T00:00:00.123450Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    }
}
async fn signed_in(fixture: &Fixture) -> Result<Session> {
    let handle = fixture.client().handle();
    let server = handle
        .add_server(&fixture.server().base_url(), UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    register(&auth, "reading").await?;
    let session = login(&auth, "reading").await?;
    fixture::online(&session).await?;
    Ok(session)
}
async fn history(session: &Session) -> Result<Arc<HistorySnapshot>> {
    let history = session.history(channel()).await?;
    history
        .page(HistoryQuery {
            limit: Some(100),
            ..HistoryQuery::default()
        })
        .await?;
    Ok(history.subscribe().await?.snapshot)
}
fn id(snapshot: &HistorySnapshot, index: usize) -> Uuid {
    snapshot.messages[index]
        .id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap()
}
async fn report(session: &Session, snapshot: &HistorySnapshot, index: usize) -> Result {
    let visible = id(snapshot, index);
    let history = session.history(channel()).await?;
    for _ in 0..10 {
        let current = history.subscribe().await?.snapshot;
        match session
            .report_visible(channel(), current.generation, current.version, &[visible])
            .await
        {
            Ok(()) => return Ok(()),
            Err(papo_sdk::ReadingError::History(papo_sdk::HistoryError::StaleSnapshot)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err("canonical history did not stabilize".into())
}
async fn reading(
    receiver: &mut watch::Receiver<Arc<ReadingSnapshot>>,
    predicate: impl Fn(&ReadState) -> bool,
) -> Result<ReadState> {
    loop {
        if let Some(state) = receiver
            .borrow_and_update()
            .channels
            .get(&channel())
            .filter(|state| predicate(state))
            .cloned()
        {
            return Ok(state);
        }
        receiver.changed().await?;
    }
}

#[test]
fn fetches_and_stale_or_detached_reports_never_acknowledge_channel() -> Result {
    let mut scenario = config();
    let seed = scenario.history[0].clone();
    scenario.history[0].count = 0;
    let fixture = Fixture::new(scenario)?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let handle = session.history(channel()).await?;
        let old = handle.subscribe().await?.snapshot;
        fixture.server().seed_history(seed).await?;
        let snapshot = history(&session).await?;
        assert!(snapshot.version > old.version);
        assert!(
            session
                .report_visible(channel(), old.generation, old.version, &[id(&snapshot, 9)])
                .await
                .is_err()
        );
        assert!(
            session
                .report_visible(
                    channel(),
                    snapshot.generation,
                    snapshot.version,
                    &[Uuid::from_u128(999)]
                )
                .await
                .is_err()
        );
        let before = session.generation();
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        fixture::wait_for(&mut status, |status| {
            status.state == ConnectionState::Online && status.generation > before
        })
        .await?;
        assert!(
            session
                .report_visible(
                    channel(),
                    snapshot.generation,
                    snapshot.version,
                    &[id(&snapshot, 9)]
                )
                .await
                .is_err()
        );
        assert_eq!(fixture.server().read_request_counts().await.uploads, 0);
        assert_eq!(session.reading_state(channel()).unwrap().local_point, None);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn visible_reports_debounce_and_coalesce_behind_exactly_one_inflight_put() -> Result {
    let fixture = Fixture::new(config())?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snapshot = history(&session).await?;
        let mut changes = session.subscribe_reading();
        let mut held = fixture
            .server()
            .hold_next_rest("PUT", &format!("/channels/{CHANNEL}/read"))
            .await?;
        for index in 0..5 {
            report(&session, &snapshot, index).await?;
        }
        held.wait_started().await?;
        assert_eq!(fixture.server().read_request_counts().await.uploads, 1);
        for index in 5..10 {
            report(&session, &snapshot, index).await?;
        }
        assert_eq!(fixture.server().read_request_counts().await.inflight, 1);
        held.release();
        reading(&mut changes, |state| {
            state
                .uploaded_point
                .is_some_and(|point| point.message_id == id(&snapshot, 9))
        })
        .await?;
        let counts = fixture.server().read_request_counts().await;
        assert_eq!((counts.uploads, counts.max_inflight), (2, 1));
        assert_eq!(
            session
                .reading_state(channel())
                .unwrap()
                .local_point
                .unwrap()
                .message_id,
            id(&snapshot, 9)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn successful_put_adopts_later_other_device_mark_instead_of_its_requested_target() -> Result {
    let fixture = Fixture::new(config())?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snapshot = history(&session).await?;
        let mut changes = session.subscribe_reading();
        let mut held = fixture
            .server()
            .hold_next_rest("PUT", &format!("/channels/{CHANNEL}/read"))
            .await?;
        report(&session, &snapshot, 4).await?;
        held.wait_started().await?;
        assert!(
            fixture
                .server()
                .advance_read_mark(
                    &session.account().user().to_string(),
                    CHANNEL,
                    &id(&snapshot, 9).to_string(),
                    "2026-02-01T00:00:00Z"
                )
                .await?
        );
        held.release();
        let state = reading(&mut changes, |state| {
            state
                .uploaded_point
                .is_some_and(|point| point.message_id == id(&snapshot, 9))
        })
        .await?;
        assert_eq!(state.local_point.unwrap().message_id, id(&snapshot, 4));
        assert_eq!(fixture.server().read_request_counts().await.uploads, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn failed_put_has_one_retry_and_preserves_durable_pending_target() -> Result {
    let mut config = config();
    config.rest.push(RestStep {
        method: "PUT".into(),
        path: format!("/channels/{CHANNEL}/read"),
        delay_ms: 0,
        failure: Some(Failure {
            status: 503,
            slug: "internal".into(),
        }),
    });
    let fixture = Fixture::new(config)?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snapshot = history(&session).await?;
        let mut changes = session.subscribe_reading();
        report(&session, &snapshot, 9).await?;
        let state = reading(&mut changes, |state| {
            state
                .uploaded_point
                .is_some_and(|point| point.message_id == id(&snapshot, 9))
        })
        .await?;
        assert_eq!(state.local_point, state.uploaded_point);
        let counts = fixture.server().read_request_counts().await;
        assert_eq!((counts.uploads, counts.max_inflight), (2, 1));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn missing_deleted_target_404_is_accepted_without_a_retry_loop() -> Result {
    let mut config = config();
    config.rest.push(RestStep {
        method: "PUT".into(),
        path: format!("/channels/{CHANNEL}/read"),
        delay_ms: 0,
        failure: Some(Failure {
            status: 404,
            slug: "not-found".into(),
        }),
    });
    let fixture = Fixture::new(config)?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snapshot = history(&session).await?;
        let mut changes = session.subscribe_reading();
        report(&session, &snapshot, 9).await?;
        let state = reading(&mut changes, |state| {
            state
                .uploaded_point
                .is_some_and(|point| point.message_id == id(&snapshot, 9))
        })
        .await?;
        assert_eq!(state.local_point, state.uploaded_point);
        assert_eq!(fixture.server().read_request_counts().await.uploads, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn full_committed_pending_point_restarts_and_uploads_without_another_visible_report() -> Result {
    let mut fixture = Fixture::new(config())?;
    let (account, point) = fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snapshot = history(&session).await?;
        fixture.server().set_gateway_accepting(false).await;
        let mut status = session.subscribe_connection();
        fixture.server().disconnect_gateways();
        fixture::wait_for(&mut status, |status| {
            matches!(status.state, ConnectionState::Backoff { .. })
        })
        .await?;
        report(&session, &snapshot, 9).await?;
        let point = session
            .reading_state(channel())
            .unwrap()
            .local_point
            .unwrap();
        assert_eq!(point.created_at_us % 1_000_000, 123_459);
        assert_eq!(fixture.server().read_request_counts().await.uploads, 0);
        Ok::<_, Box<dyn std::error::Error>>((session.account(), point))
    })??;
    fixture.close_client()?;
    fixture.reopen()?;
    fixture.run(async {
        fixture.server().set_gateway_accepting(true).await;
        let session = fixture.client().handle().session(account)?;
        fixture::online(&session).await?;
        let mut changes = session.subscribe_reading();
        let state = reading(&mut changes, |state| state.uploaded_point == Some(point)).await?;
        assert_eq!(state.local_point, Some(point));
        assert_eq!(fixture.server().read_request_counts().await.uploads, 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
