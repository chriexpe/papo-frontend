//! Loopback coverage for `Session::mark_all_read` and live message unread tracking on unopened channels.

use std::{sync::Arc, time::Duration};

use papo_api::{HistoryQuery, Uuid, wire::Message};
use papo_sdk::{BadgesSnapshot, ReadingSnapshot, Session, UnixMicros, UnreadCount};
use papo_test_server::{Config, HistorySeed};
use tokio::sync::watch;

use super::fixture::{self, Fixture, Result, login, register};

const CH_OPEN: &str = "10000000-0000-4000-8000-000000000011";
const CH_CLOSED: &str = "10000000-0000-4000-8000-000000000012";
const CH_READ: &str = "10000000-0000-4000-8000-000000000013";
const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";

fn multi_channel_config() -> Config {
    Config {
        history: vec![
            HistorySeed {
                channel_id: CH_OPEN.into(),
                author_id: AUTHOR.into(),
                first_message_id: "70000000-0000-4000-8000-000000000010".into(),
                count: 3,
                first_created_at: "2026-01-01T00:00:00.100000Z".into(),
                timestamp_step_us: 1,
                equal_timestamp_count: 1,
            },
            HistorySeed {
                channel_id: CH_CLOSED.into(),
                author_id: AUTHOR.into(),
                first_message_id: "70000000-0000-4000-8000-000000000020".into(),
                count: 2,
                first_created_at: "2026-01-01T00:00:00.200000Z".into(),
                timestamp_step_us: 1,
                equal_timestamp_count: 1,
            },
            HistorySeed {
                channel_id: CH_READ.into(),
                author_id: AUTHOR.into(),
                first_message_id: "70000000-0000-4000-8000-000000000030".into(),
                count: 1,
                first_created_at: "2026-01-01T00:00:00.300000Z".into(),
                timestamp_step_us: 1,
                equal_timestamp_count: 1,
            },
        ],
        ..Config::default()
    }
}

async fn wait_uploaded(
    rx: &mut watch::Receiver<Arc<ReadingSnapshot>>,
    expected: &[(Uuid, Uuid)],
) -> Result {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let image = rx.borrow_and_update().clone();
            if expected.iter().all(|(ch, msg)| {
                image
                    .channels
                    .get(ch)
                    .and_then(|s| s.uploaded_point)
                    .is_some_and(|p| p.message_id == *msg)
            }) {
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
            rx.changed().await?;
        }
    })
    .await??;
    Ok(())
}

async fn wait_badges(
    rx: &mut watch::Receiver<Arc<BadgesSnapshot>>,
    predicate: impl Fn(&BadgesSnapshot) -> bool,
) -> Result {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if predicate(&rx.borrow_and_update()) {
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
            rx.changed().await?;
        }
    })
    .await??;
    Ok(())
}

async fn open_channel(session: &Session, channel: Uuid) -> Result<Uuid> {
    let hist = session.history(channel).await?;
    hist.page(HistoryQuery {
        limit: Some(10),
        ..HistoryQuery::default()
    })
    .await?;
    let snap = hist.subscribe().await?.snapshot;
    Ok(snap
        .messages
        .last()
        .unwrap()
        .id
        .as_deref()
        .unwrap()
        .parse()?)
}

#[test]
fn mark_all_read_advances_unread_server_channels_skips_read_and_leaves_dms_untouched() -> Result {
    let fixture = Fixture::new(multi_channel_config())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = handle.auth(server)?;
        register(&auth, "markall-a").await?;
        register(&auth, "markall-b").await?;
        let session = login(&auth, "markall-a").await?;
        let peer = login(&auth, "markall-b").await?;
        fixture::online(&session).await?;
        fixture::online(&peer).await?;

        let open_id: Uuid = CH_OPEN.parse()?;
        let closed_id: Uuid = CH_CLOSED.parse()?;
        let read_id: Uuid = CH_READ.parse()?;

        // Pre-mark CH_READ as read via the regular observer path.
        let read_msg = open_channel(&session, read_id).await?;
        let read_snap = session.history(read_id).await?.subscribe().await?.snapshot;
        session
            .reading_observer()
            .report_visible(
                read_id,
                read_snap.generation,
                read_snap.version,
                &[read_msg],
                papo_sdk::VisibilityContinuity::Continuous,
            )
            .await?;
        let mut reading_rx = session.subscribe_reading();
        wait_uploaded(&mut reading_rx, &[(read_id, read_msg)]).await?;
        assert_eq!(fixture.server().read_request_counts().await.uploads, 1);

        // Open CH_OPEN so one unread channel has a snapshot and CH_CLOSED does not.
        let open_last = open_channel(&session, open_id).await?;
        let dm_id: Uuid = session
            .operations()
            .open_direct_conversation(peer.account().user())
            .await?
            .value
            .id
            .parse()?;

        let mut badges = session.subscribe_badges();
        wait_badges(&mut badges, |snap| {
            snap.channels
                .get(&open_id)
                .is_some_and(|b| b.has_unread == Some(true))
                && snap
                    .channels
                    .get(&closed_id)
                    .is_some_and(|b| b.has_unread == Some(true))
        })
        .await?;

        session.mark_all_read().await?;

        let closed_last: Uuid = "70000000-0000-4000-8000-000000000021".parse()?;
        assert_eq!(
            session
                .reading_state(open_id)
                .and_then(|s| s.local_point)
                .map(|p| p.message_id),
            Some(open_last)
        );
        assert_eq!(
            session
                .reading_state(closed_id)
                .and_then(|s| s.local_point)
                .map(|p| p.message_id),
            Some(closed_last)
        );
        assert_eq!(
            session.reading_state(open_id).map(|s| s.unread_count),
            Some(UnreadCount::Exact(0))
        );
        assert!(
            session
                .reading_state(dm_id)
                .and_then(|s| s.local_point)
                .is_none(),
            "mark_all_read must not touch direct conversations"
        );

        wait_uploaded(
            &mut reading_rx,
            &[(open_id, open_last), (closed_id, closed_last)],
        )
        .await?;
        assert_eq!(fixture.server().read_request_counts().await.uploads, 3);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_message_in_unopened_channel_marks_unread_and_mark_all_read_clears_it() -> Result {
    let config = Config {
        history: vec![HistorySeed {
            channel_id: CH_CLOSED.into(),
            author_id: AUTHOR.into(),
            first_message_id: "70000000-0000-4000-8000-000000000040".into(),
            count: 1,
            first_created_at: "2026-01-01T00:00:00.100000Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    };
    let fixture = Fixture::new(config)?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = handle.auth(server)?;
        register(&auth, "live-head").await?;
        let session = login(&auth, "live-head").await?;
        fixture::online(&session).await?;

        let ch_id: Uuid = CH_CLOSED.parse()?;
        session.mark_all_read().await?;
        let mut reading_rx = session.subscribe_reading();
        let initial_id: Uuid = "70000000-0000-4000-8000-000000000040".parse()?;
        wait_uploaded(&mut reading_rx, &[(ch_id, initial_id)]).await?;

        let mut badges = session.subscribe_badges();
        let live_id = "70000000-0000-4000-8000-000000000099";
        fixture
            .server()
            .publish_message(Message {
                id: Some(live_id.into()),
                channel_id: Some(CH_CLOSED.into()),
                author_id: Some(AUTHOR.into()),
                content: Some("live ping".into()),
                created_at: Some("2026-01-01T00:00:01.000000Z".into()),
                ..Default::default()
            })
            .await?;

        wait_badges(&mut badges, |snap| {
            snap.channels
                .get(&ch_id)
                .is_some_and(|b| b.has_unread == Some(true))
        })
        .await?;

        session.mark_all_read().await?;
        assert_eq!(
            session
                .reading_state(ch_id)
                .and_then(|s| s.local_point)
                .map(|p| p.message_id),
            Some(live_id.parse()?)
        );
        wait_badges(&mut badges, |snap| {
            snap.channels
                .get(&ch_id)
                .is_some_and(|b| b.has_unread != Some(true))
        })
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
