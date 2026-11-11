//! Loopback lifetimes and public timeline observation; production SDK/UI internals are never accessed.

use std::{error::Error, future::Future, sync::Arc, time::Duration};

use futures_util::{FutureExt, StreamExt};
use papo_sdk::{
    AuthService, Backoff, Client, ClientConfig, ConnectionState, DataRoot, Jitter, LoginRequest,
    Message, RegisterRequest, Session, UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use papo_sdk_ui::{TimelineEvent, TimelineItem, TimelineSubscription};
use papo_test_server::{Config, HistorySeed, RestStep, TestServer};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
pub const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";
const FIRST: &str = "70000000-0000-4000-8000-000000000001";

fn recipe(count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: AUTHOR.into(),
        first_message_id: FIRST.into(),
        count,
        first_created_at: "2026-01-01T01:00:00.123456+01:00".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

pub fn channel() -> Uuid {
    Uuid::parse_str(CHANNEL).unwrap()
}

pub fn config() -> papo_sdk_ui::TimelineConfig {
    papo_sdk_ui::TimelineConfig {
        page_size: 4,
        subscriber_capacity: 8,
        ..Default::default()
    }
}

struct NoJitter;
impl Jitter for NoJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}

pub struct Fixture {
    client: Option<Client>,
    server: Option<TestServer>,
    runtime: Option<tokio::runtime::Runtime>,
    _profile: tempfile::TempDir,
    pub messages: Vec<Message>,
}

impl Fixture {
    pub fn new(count: usize, rest: Vec<RestStep>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let server = runtime.block_on(TestServer::start(Config {
            history: vec![recipe(0)],
            rest,
            ..Config::default()
        }))?;
        let profile = tempfile::tempdir()?;
        let client = Client::open(
            DataRoot::new(profile.path()),
            Arc::new(ScriptedSecretStore::default()),
            ClientConfig {
                insecure_loopback: true,
                jitter: Arc::new(NoJitter),
                backoff: Backoff {
                    initial: Duration::from_secs(2),
                    maximum: Duration::from_secs(2),
                },
                ..ClientConfig::default()
            },
        )?;
        Ok(Self {
            client: Some(client),
            server: Some(server),
            runtime: Some(runtime),
            _profile: profile,
            messages: recipe(count).messages()?,
        })
    }

    pub fn client(&self) -> &Client {
        self.client.as_ref().unwrap()
    }
    pub fn server(&self) -> &TestServer {
        self.server.as_ref().unwrap()
    }

    pub fn run<F: Future>(&self, work: F) -> Result<F::Output> {
        Ok(self
            .client()
            .block_on(async { tokio::time::timeout(Duration::from_secs(30), work).await })??)
    }

    pub async fn signed_in(&self, username: &str) -> Result<Session> {
        let handle = self.client().handle();
        let server = handle
            .add_server(&self.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        auth.register(RegisterRequest {
            username: username.into(),
            password: password(),
        })
        .await?;
        let session = login(&auth, username).await?;
        online(&session).await?;
        // Silent seeding after reconcile keeps the first UI page an explicit public fetch.
        self.server()
            .seed_messages(CHANNEL, self.messages.clone())
            .await?;
        Ok(session)
    }

    pub fn finish(mut self) -> Result {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result {
        let client = self
            .client
            .take()
            .map(|mut client| client.shutdown())
            .transpose();
        let server = match (self.server.take(), self.runtime.as_ref()) {
            (Some(server), Some(runtime)) => runtime.block_on(server.shutdown()),
            _ => Ok(()),
        };
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(3));
        }
        if let Some(report) = client? {
            assert!(report.reads_drained && report.state.finished && report.shared.finished);
            assert!(!report.state.deadline_reached && !report.shared.deadline_reached);
        }
        server?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn password() -> String {
    std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect()
}

pub async fn login(auth: &AuthService, username: &str) -> Result<Session> {
    Ok(auth
        .login(
            LoginRequest {
                username: username.into(),
                password: password(),
            },
            UnixMicros(0),
        )
        .await?)
}

pub async fn online(session: &Session) -> Result {
    let mut status = session.subscribe_connection();
    loop {
        let current = *status.borrow_and_update();
        if current.state == ConnectionState::Online && current.generation == session.generation() {
            return Ok(());
        }
        status.changed().await?;
    }
}

pub fn ids(subscription: &TimelineSubscription) -> Vec<Uuid> {
    subscription
        .items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

pub fn payload(subscription: &TimelineSubscription, id: Uuid) -> Option<&Arc<Message>> {
    subscription
        .items
        .iter()
        .find_map(|item| match item.as_ref() {
            TimelineItem::Message {
                id: current,
                message,
            } if *current == id => Some(message),
            _ => None,
        })
}

pub async fn until(
    subscription: &mut TimelineSubscription,
    predicate: impl Fn(&TimelineSubscription) -> bool,
) -> Result {
    while !predicate(subscription) {
        match subscription
            .stream
            .next()
            .await
            .ok_or("timeline ended before expected update")?
        {
            TimelineEvent::Update(update) => {
                for diff in update.diffs {
                    diff.apply(&mut subscription.items);
                }
                subscription.version = update.version;
                subscription.generation = update.generation;
            }
            TimelineEvent::Closed(_) => return Err("timeline closed before expected update".into()),
            _ => return Err("unknown timeline event".into()),
        }
    }
    Ok(())
}

pub async fn closed(
    subscription: &mut TimelineSubscription,
) -> Result<papo_sdk::HistoryClosedReason> {
    loop {
        match subscription
            .stream
            .next()
            .await
            .ok_or("timeline ended without typed closure")?
        {
            TimelineEvent::Update(update) => {
                for diff in update.diffs {
                    diff.apply(&mut subscription.items);
                }
            }
            TimelineEvent::Closed(reason) => {
                assert!(matches!(
                    subscription.stream.next().now_or_never(),
                    Some(None)
                ));
                return Ok(reason);
            }
            _ => return Err("unknown timeline event".into()),
        }
    }
}
