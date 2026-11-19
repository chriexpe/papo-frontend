//! Proves that reconnect, gateway and operation refresh admit or reject the same wire input
//! over loopback, keeping the last-good snapshot and the SQLite history cache. Pure
//! per-field rules stay in the core `resource_admission` unit tests.

#![cfg(feature = "test-support")]

#[path = "resource_admission_cases/acceptance.rs"]
mod acceptance;
#[path = "resource_admission_cases/defects.rs"]
mod defects;
mod fixture;
#[path = "resource_admission_cases/rejection.rs"]
mod rejection;

use fixture::{Fixture, Result, login, register, wait_for};
use papo_api::{HistoryOrder, HistoryQuery, Uuid};
use papo_sdk::{
    Backoff, ConnectionState, ConnectionStatus, DataRoot, ResourceSnapshot, Session, UnixMicros,
};
use papo_test_server::{Config, HistorySeed, TestServer};
use std::{path::PathBuf, sync::Arc, time::Duration};

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const SEEDED_MESSAGES: i64 = 5;

/// The first retry waits half of `initial` (no jitter). That window is long enough to
/// inspect the cache between a rejection and the next connection attempt.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

fn config() -> Result<Config> {
    Ok(Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: "20000000-0000-4000-8000-000000000001".into(),
            first_message_id: "70000000-0000-4000-8000-000000000001".into(),
            count: usize::try_from(SEEDED_MESSAGES)?,
            first_created_at: "2026-01-01T01:00:00.123456Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    })
}

fn fixture() -> Result<Fixture> {
    Fixture::with_backoff(
        config()?,
        Backoff {
            initial: BACKOFF_INITIAL,
            maximum: Duration::from_secs(16),
        },
    )
}

/// A signed-in account whose seeded channel history is already in its SQLite cache.
struct Account<'a> {
    fixture: &'a Fixture,
    session: Session,
    cache: PathBuf,
}

impl<'a> Account<'a> {
    async fn start(fixture: &'a Fixture, username: &str) -> Result<Self> {
        let handle = fixture.client().handle();
        let server_id = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server_id)?;
        register(&auth, username).await?;
        let session = login(&auth, username).await?;
        fixture::online(&session).await?;
        let history = session.history(Uuid::parse_str(CHANNEL)?).await?;
        let page = history
            .page(HistoryQuery {
                order: HistoryOrder::Desc,
                limit: Some(100),
                ..HistoryQuery::default()
            })
            .await?;
        assert_eq!(i64::try_from(page.messages.len())?, SEEDED_MESSAGES);
        drop(history);
        let cache = DataRoot::new(fixture.profile.path()).account_cache(session.account());
        let account = Self {
            fixture,
            session,
            cache,
        };
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        Ok(account)
    }

    fn server(&self) -> &TestServer {
        self.fixture.server()
    }

    fn own(&self) -> Uuid {
        self.session.account().user()
    }

    fn snapshot(&self) -> Arc<ResourceSnapshot> {
        self.session.subscribe_resources().borrow().clone()
    }

    fn status(&self) -> ConnectionStatus {
        *self.session.subscribe_connection().borrow()
    }

    /// Reads the account cache read-only, so a deleted file fails instead of being recreated.
    async fn cached_messages(&self) -> Result<i64> {
        let path = self.cache.clone();
        Ok(tokio::task::spawn_blocking(move || {
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        })
        .await??)
    }

    async fn script(&self, method: &str, path: &str, body: serde_json::Value) -> Result {
        Ok(self
            .server()
            .queue_operation_response(method, path, 200, body)
            .await?)
    }
}

async fn online_at(
    session: &Session,
    accept: impl Fn(ConnectionStatus) -> bool,
) -> Result<ConnectionStatus> {
    let mut status = session.subscribe_connection();
    wait_for(&mut status, |status| {
        status.state == ConnectionState::Online && accept(status)
    })
    .await
}

/// Identity and display digest of every authoritative group, ignoring publication counters
/// and the live fields a cache-loaded image strips.
fn digest(snapshot: &ResourceSnapshot) -> Vec<String> {
    let mut rows = Vec::new();
    if let Some(server) = &snapshot.server {
        rows.push(format!("server:{:?}:{:?}", server.id, server.owner_id));
    }
    for channel in snapshot.channels.iter() {
        let overwrites = channel.permissions.as_ref().map(Vec::len);
        rows.push(format!(
            "channel:{:?}:{:?}:{overwrites:?}",
            channel.id, channel.name
        ));
    }
    rows.extend(
        snapshot
            .roles
            .iter()
            .map(|role| format!("role:{:?}", role.id)),
    );
    for member in snapshot.members.iter() {
        let roles: Vec<_> = member.roles.iter().flatten().map(|role| &role.id).collect();
        rows.push(format!(
            "member:{:?}:{:?}:{roles:?}",
            member.id, member.nickname
        ));
    }
    rows.extend(snapshot.dms.iter().map(|dm| format!("dm:{}", dm.id)));
    rows.extend(
        snapshot
            .blocks
            .iter()
            .map(|user| format!("block:{:?}", user.id)),
    );
    rows.sort();
    rows
}
