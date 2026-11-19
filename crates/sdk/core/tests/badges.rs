//! Public badge evidence and committed policy over loopback and real SQLite; pure classification stays beside policy.

#![cfg(feature = "test-support")]

#[path = "badge_cases/access.rs"]
mod access;
#[path = "badge_cases/classification.rs"]
mod classification;
#[expect(
    dead_code,
    reason = "this fixture suite does not exercise credential restart helpers"
)]
mod fixture;
#[path = "badge_cases/settings.rs"]
mod settings;

use fixture::{Fixture, Result, login, register};
use papo_api::{HistoryQuery, wire::Message};
use papo_sdk::{BadgesSnapshot, ChannelBadges, Session, UnixMicros, UnreadCount, Uuid};
use papo_test_server::{Config, HistorySeed};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";
const ROLE: &str = "30000000-0000-4000-8000-000000000001";

fn channel() -> Uuid {
    CHANNEL.parse().unwrap()
}

fn config() -> Config {
    Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: AUTHOR.into(),
            first_message_id: message_id(0).to_string(),
            count: 0,
            first_created_at: "2026-01-01T00:00:00Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    }
}

fn message_id(index: u128) -> Uuid {
    Uuid::from_u128(0x7000_0000_0000_4000_8000_0000_0000_0000 + index)
}

fn message(index: u32, channel: Uuid, author: Uuid, content: &str) -> Message {
    Message {
        id: Some(message_id(u128::from(index)).to_string()),
        channel_id: Some(channel.to_string()),
        author_id: Some(author.to_string()),
        content: Some(content.into()),
        created_at: Some(format!("2026-01-01T00:00:00.{index:06}Z")),
        ..Message::default()
    }
}

async fn account(f: &Fixture, username: &str) -> Result<Session> {
    let client = f.client().handle();
    let saved = client.subscribe_registry();
    let existing = saved
        .borrow()
        .servers
        .first()
        .map(|server| server.address.id());
    let server = match existing {
        Some(server) => server,
        None => {
            client
                .add_server(&f.server().base_url(), UnixMicros(0))
                .await?
        }
    };
    let auth = client.auth(server)?;
    register(&auth, username).await?;
    let session = login(&auth, username).await?;
    fixture::online(&session).await?;
    Ok(session)
}

async fn badge(
    changes: &mut watch::Receiver<Arc<BadgesSnapshot>>,
    channel: Uuid,
    predicate: impl Fn(&ChannelBadges) -> bool,
) -> Result<Arc<ChannelBadges>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = changes
                .borrow_and_update()
                .channels
                .get(&channel)
                .filter(|value| predicate(value))
                .cloned();
            if let Some(current) = current {
                return Ok::<_, Box<dyn std::error::Error>>(current);
            }
            changes.changed().await?;
        }
    })
    .await
    .map_err(|error| -> Box<dyn std::error::Error> {
        format!(
            "badge predicate timeout, observed {:?}: {error}",
            changes.borrow().channels.get(&channel)
        )
        .into()
    })?
}

async fn count(
    changes: &mut watch::Receiver<Arc<BadgesSnapshot>>,
    channel: Uuid,
    expected: UnreadCount,
) -> Result<Arc<ChannelBadges>> {
    badge(changes, channel, |value| value.mention_count == expected)
        .await
        .map_err(|error| {
            format!(
                "expected {expected:?}, observed {:?}: {error}",
                changes.borrow().channels.get(&channel)
            )
            .into()
        })
}

async fn read_through(session: &Session, channel: Uuid, visible: Uuid) -> Result {
    let history = session.history(channel).await?;
    let observer = session.reading_observer();
    for _ in 0..10 {
        let snapshot = history.subscribe().await?.snapshot;
        match observer
            .report_visible(
                channel,
                snapshot.generation,
                snapshot.version,
                &[visible],
                papo_sdk::VisibilityContinuity::Continuous,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(papo_sdk::ReadingError::History(papo_sdk::HistoryError::StaleSnapshot)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err("canonical history did not stabilize".into())
}

async fn load_and_anchor(
    f: &Fixture,
    session: &Session,
    channel: Uuid,
    messages: Vec<Message>,
) -> Result {
    f.server()
        .seed_messages(&channel.to_string(), messages)
        .await?;
    session
        .history(channel)
        .await?
        .page(HistoryQuery {
            limit: Some(100),
            ..HistoryQuery::default()
        })
        .await?;
    read_through(session, channel, message_id(1)).await
}

async fn event(
    session: &Session,
    kind: &str,
    work: impl std::future::Future<Output = Result>,
) -> Result {
    let mut events = session.subscribe_events();
    work.await?;
    loop {
        if events.next().await?.value.kind() == kind {
            return Ok(());
        }
    }
}
