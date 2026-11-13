//! Owns loopback integration lifetimes and bounded state waits; production services own SDK policy.

use std::{error::Error, future::Future, sync::Arc, time::Duration};

use papo_api::wire::{LoginRequest, RegisterRequest};
use papo_sdk::{
    AuthService, Backoff, Client, ClientConfig, ConnectionState, ConnectionStatus, DataRoot,
    Generation, Jitter, Session, UnixMicros, secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, TestServer};
use tokio::{runtime::Runtime, sync::watch};

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

pub struct Fixture {
    client: Option<Client>,
    pub server: Option<TestServer>,
    pub store: Arc<ScriptedSecretStore>,
    pub(super) profile: tempfile::TempDir,
    server_runtime: Option<Runtime>,
    pub(super) config: ClientConfig,
}

struct NoJitter;
impl Jitter for NoJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}

impl Fixture {
    pub fn new(server_config: Config) -> Result<Self> {
        // Restart tests must retain the server while destroying the client runtime.
        // This runtime serves only the fixture; no SDK work is driven by it.
        let server_runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let server = server_runtime.block_on(TestServer::start(server_config))?;
        let profile = tempfile::tempdir()?;
        let store = Arc::new(ScriptedSecretStore::default());
        let config = ClientConfig {
            insecure_loopback: true,
            jitter: Arc::new(NoJitter),
            backoff: Backoff {
                initial: Duration::from_secs(2),
                maximum: Duration::from_secs(2),
            },
            ..ClientConfig::default()
        };
        let client = Client::open(DataRoot::new(profile.path()), store.clone(), config.clone())?;
        Ok(Self {
            client: Some(client),
            server: Some(server),
            store,
            profile,
            server_runtime: Some(server_runtime),
            config,
        })
    }

    pub fn client(&self) -> &Client {
        self.client.as_ref().expect("fixture client exists")
    }

    pub fn server(&self) -> &TestServer {
        self.server.as_ref().expect("fixture server exists")
    }

    pub fn run<F: Future>(&self, work: F) -> Result<F::Output> {
        Ok(self
            .client()
            .block_on(async { tokio::time::timeout(Duration::from_secs(30), work).await })??)
    }

    pub fn close_client(&mut self) -> Result {
        if let Some(mut client) = self.client.take() {
            check_shutdown(&client.shutdown()?);
        }
        Ok(())
    }

    pub fn reopen(&mut self) -> Result {
        self.close_client()?;
        self.client = Some(Client::open(
            DataRoot::new(self.profile.path()),
            self.store.clone(),
            self.config.clone(),
        )?);
        Ok(())
    }

    pub fn finish(mut self) -> Result {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result {
        let client = match self.client.take() {
            Some(mut client) => client.shutdown().map(|report| check_shutdown(&report)),
            None => Ok(()),
        };
        let server = match (self.server.take(), self.server_runtime.as_ref()) {
            (Some(server), Some(runtime)) => runtime.block_on(server.shutdown()),
            _ => Ok(()),
        };
        if let Some(runtime) = self.server_runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(3));
        }
        client?;
        server?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn check_shutdown(report: &papo_sdk::StorageShutdown) {
    assert!(report.reads_drained);
    assert!(report.state.finished);
    assert!(report.shared.finished);
    assert!(!report.state.deadline_reached);
    assert!(!report.shared.deadline_reached);
}

pub fn password(variant: u8) -> String {
    std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + (index + variant) % 26)))
        .chain(std::iter::once('!'))
        .collect()
}

pub async fn register(auth: &AuthService, username: &str) -> Result {
    auth.register(RegisterRequest {
        username: username.into(),
        password: password(0),
    })
    .await?;
    Ok(())
}

pub async fn login(auth: &AuthService, username: &str) -> Result<Session> {
    Ok(auth
        .login(
            LoginRequest {
                username: username.into(),
                password: password(0),
            },
            UnixMicros(0),
        )
        .await?)
}

pub async fn wait_for(
    status: &mut watch::Receiver<ConnectionStatus>,
    predicate: impl Fn(ConnectionStatus) -> bool,
) -> Result<ConnectionStatus> {
    loop {
        let current = *status.borrow_and_update();
        if predicate(current) {
            return Ok(current);
        }
        status.changed().await?;
    }
}

pub async fn online(session: &Session) -> Result<Generation> {
    let mut status = session.subscribe_connection();
    Ok(wait_for(&mut status, |status| {
        status.state == ConnectionState::Online
    })
    .await?
    .generation)
}
