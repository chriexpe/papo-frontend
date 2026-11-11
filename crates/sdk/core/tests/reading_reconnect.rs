//! Measures bounded unread-channel reconnect work over TCP; leaves storage fairness proofs to existing tests.

#![cfg(feature = "test-support")]

mod fixture;

use fixture::{Fixture, Result, login, register};
use papo_sdk::{ConnectionState, UnixMicros, UnreadCount, Uuid};
use papo_test_server::{Config, HistorySeed};
use std::time::Instant;

const CHANNELS: usize = 3;

fn recipe(index: usize, burst: bool) -> HistorySeed {
    HistorySeed {
        channel_id: Uuid::from_u128(100 + index as u128).to_string(),
        author_id: Uuid::from_u128(999).to_string(),
        first_message_id: Uuid::from_u128(
            10_000 + (index as u128) * 10_000 + if burst { 1000 } else { 0 },
        )
        .to_string(),
        count: if burst { 401 } else { 60 },
        first_created_at: if burst {
            "2026-01-01T00:00:01.123456Z"
        } else {
            "2026-01-01T00:00:00.123456Z"
        }
        .into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 3,
    }
}

#[test]
fn three_unread_channels_without_open_handles_reconnect_with_twelve_page_budget() -> Result {
    let mut fixture = Fixture::new(Config {
        history: (0..CHANNELS).map(|index| recipe(index, false)).collect(),
        ..Config::default()
    })?;
    let account = fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle.add_server(&fixture.server().base_url(), UnixMicros(0)).await?;
        let auth = handle.auth(server)?;
        register(&auth, "reconnect-cost").await?;
        let session = login(&auth, "reconnect-cost").await?;
        fixture::online(&session).await?;
        let initial = fixture.server().read_request_counts().await;
        assert_eq!((initial.history_pages, initial.history_rows), (3, 180));
        for index in 0..CHANNELS {
            let seed = recipe(index, false);
            fixture.server().set_read_mark(&session.account().user().to_string(), &seed.channel_id, &seed.first_message_id, "2026-02-01T00:00:00Z").await?;
        }
        fixture.server().set_gateway_accepting(false).await;
        let mut connection = session.subscribe_connection();
        fixture.server().disconnect_gateways();
        let offline = fixture::wait_for(&mut connection, |status| matches!(status.state, ConnectionState::Backoff { .. })).await?;
        for index in 0..CHANNELS {
            fixture.server().seed_history(recipe(index, true)).await?;
        }
        let before = fixture.server().read_request_counts().await;
        let started = Instant::now();
        fixture.server().set_gateway_accepting(true).await;
        session.reconnect()?;
        let online = fixture::wait_for(&mut connection, |status| status.state == ConnectionState::Online && status.generation > offline.generation).await?;
        let elapsed_ms = started.elapsed().as_millis();
        let after = fixture.server().read_request_counts().await;
        let pages = after.history_pages - before.history_pages;
        let rows = after.history_rows - before.history_rows;
        assert_eq!((pages, rows), (CHANNELS * 4, CHANNELS * 400));
        assert_eq!(after.channel_lists - before.channel_lists, 1);
        assert_eq!(after.uploads, 0);
        for index in 0..CHANNELS {
            let seed = recipe(index, false);
            let state = session.reading_state(seed.channel_id.parse()?).ok_or("missing unread state")?;
            assert_eq!(state.uploaded_point.map(|point| point.message_id.to_string()), Some(seed.first_message_id));
            assert!(matches!(state.unread_count, UnreadCount::AtLeast(_)));
            assert!(state.local_point.is_none());
        }
        eprintln!("Unread reconnect observation: channels={CHANNELS} initial_pages={} initial_rows={} reconnect_pages={pages} reconnect_rows={rows} elapsed_ms={elapsed_ms} generation={}", initial.history_pages, initial.history_rows, online.generation.get());
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        fixture::online(&session).await?;
        for index in 0..CHANNELS {
            let seed = recipe(index, false);
            let state = session
                .reading_state(seed.channel_id.parse()?)
                .ok_or("missing restored unread state")?;
            assert_eq!(
                state
                    .uploaded_point
                    .map(|point| point.message_id.to_string()),
                Some(seed.first_message_id)
            );
            assert!(state.local_point.is_none());
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
