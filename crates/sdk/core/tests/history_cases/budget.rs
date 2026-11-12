//! Public idle eviction, raw-subscriber/inflight pinning and cold-channel revocation over loopback.

use super::fixture;
use fixture::{Fixture, Result, login, register, wait_for};
use papo_api::{HistoryQuery, Uuid, wire::Channel};
use papo_sdk::{ConnectionState, HistoryError, Session, UnixMicros};
use papo_test_server::{Config, HistorySeed};

fn seed(index: usize, count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: Uuid::from_u128(10_000 + index as u128).to_string(),
        author_id: Uuid::from_u128(999).to_string(),
        first_message_id: Uuid::from_u128(100_000 + index as u128).to_string(),
        count,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 1,
    }
}
async fn signed_in(fixture: &Fixture) -> Result<Session> {
    let handle = fixture.client().handle();
    let server = handle
        .add_server(&fixture.server().base_url(), UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    register(&auth, "historybudget").await?;
    let session = login(&auth, "historybudget").await?;
    fixture::online(&session).await?;
    Ok(session)
}
async fn reconnect(fixture: &Fixture, session: &Session) -> Result {
    let before = session.generation();
    let mut connection = session.subscribe_connection();
    fixture.server().set_gateway_accepting(true).await;
    session.reconnect()?;
    wait_for(&mut connection, |status| {
        status.state == ConnectionState::Online && status.generation > before
    })
    .await?;
    Ok(())
}

#[test]
fn raw_watch_pins_history_and_evicted_channel_revocation_cannot_restore_old_rows_on_regrant()
-> Result {
    let fixture = Fixture::new(Config {
        history: (0..20).map(|index| seed(index, 1)).collect(),
        ..Config::default()
    })?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let channel: Uuid = seed(0, 1).channel_id.parse()?;
        let handle = session.history(channel).await?;
        let watch = handle.subscribe().await?;
        drop(handle);
        for index in 1..20 {
            drop(session.history(seed(index, 1).channel_id.parse()?).await?);
            let report = session.trim_history().await?;
            assert!(report.idle_channels <= 8);
            assert!(report.pinned_channels >= 1);
        }
        let old_snapshot = watch.snapshot.clone();
        drop(watch);
        let report = session.trim_history().await?;
        assert_eq!(report.pinned_channels, 0);
        assert!(report.work.evictions > 0);
        assert_eq!(old_snapshot.messages.len(), 1);
        let fresh = session.history(channel).await?;
        assert_ne!(
            fresh.subscribe().await?.snapshot.version,
            old_snapshot.version
        );
        assert!(matches!(
            session
                .report_visible(
                    channel,
                    old_snapshot.generation,
                    old_snapshot.version,
                    &[old_snapshot.messages[0].id.as_deref().unwrap().parse()?]
                )
                .await,
            Err(papo_sdk::ReadingError::History(HistoryError::StaleSnapshot))
        ));
        drop(fresh);
        for index in 1..20 {
            drop(session.history(seed(index, 1).channel_id.parse()?).await?);
            session.trim_history().await?;
        }
        fixture.server().set_gateway_accepting(false).await;
        let mut connection = session.subscribe_connection();
        fixture.server().disconnect_gateways();
        wait_for(&mut connection, |status| {
            matches!(status.state, ConnectionState::Backoff { .. })
        })
        .await?;
        assert!(fixture.server().delete_channel(&channel.to_string()).await);
        reconnect(&fixture, &session).await?;
        assert!(matches!(
            session.history(channel).await,
            Err(HistoryError::Unverified)
        ));
        fixture
            .server()
            .set_channel(Channel {
                id: Some(channel.to_string()),
                name: Some("regranted".into()),
                r#type: Some("text".into()),
                position: Some(0),
                ..Channel::default()
            })
            .await?;
        reconnect(&fixture, &session).await?;
        let fresh = session.history(channel).await?;
        assert!(fresh.subscribe().await?.snapshot.messages.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn held_page_remains_pinned_after_caller_drops_handle_and_other_channels_are_evicted() -> Result {
    let fixture = Fixture::new(Config {
        history: (0..20).map(|index| seed(index, 0)).collect(),
        ..Config::default()
    })?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        fixture.server().seed_history(seed(0, 1)).await?;
        let channel = seed(0, 1).channel_id;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{channel}/messages"))
            .await?;
        let history = session.history(channel.parse()?).await?;
        let pending = tokio::spawn(async move { history.page(HistoryQuery::default()).await });
        hold.wait_started().await?;
        for index in 1..20 {
            drop(session.history(seed(index, 0).channel_id.parse()?).await?);
            let report = session.trim_history().await?;
            assert!(report.pinned_channels >= 1);
            assert!(report.idle_channels <= 8);
        }
        hold.release();
        assert_eq!(pending.await??.messages.len(), 1);
        let history = session.history(channel.parse()?).await?;
        assert_eq!(history.subscribe().await?.snapshot.messages.len(), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
