//! Deterministic connector scripts and paused-clock retry tests; never combines timers with TCP.

use super::*;
use crate::{
    AccountId, ClientConfig, Jitter, ServerAddress,
    secret::{CredentialGeneration, SecretKey},
};
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

struct ZeroJitter;
impl Jitter for ZeroJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}
struct Script {
    steps: Mutex<VecDeque<Result<Option<Event>, ClientError>>>,
    attempts: AtomicU64,
}
struct ScriptSocket(Option<Result<Option<Event>, ClientError>>);
impl Socket for ScriptSocket {
    fn next(&mut self) -> IoFuture<'_, Option<Event>> {
        Box::pin(async move {
            if let Some(step) = self.0.take() {
                step
            } else {
                std::future::pending().await
            }
        })
    }
}
impl Connector for Script {
    fn connect<'a>(&'a self, _: &'a SessionInner, _: &'a Epoch) -> IoFuture<'a, Box<dyn Socket>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(ScriptSocket(self.steps.lock().unwrap().pop_front())) as Box<dyn Socket>)
        })
    }
}
fn session() -> Arc<SessionInner> {
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), papo_api::Uuid::nil());
    SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::new(AtomicU64::new(1)),
        tokio_util::sync::CancellationToken::new(),
        AuthState::Ready,
    )
}
#[tokio::test(start_paused = true)]
async fn close_and_heartbeat_failure_enter_backoff_and_replace_generation() {
    for step in [
        Ok(Some(Event::Closed(papo_api::gateway::CloseReason::Normal))),
        Err(ClientError::Gateway(
            papo_api::gateway::Error::HeartbeatTimeout,
        )),
    ] {
        let session = session();
        let old = session.epochs.current();
        let config = ClientConfig {
            jitter: Arc::new(ZeroJitter),
            ..ClientConfig::default()
        };
        let script = Script {
            steps: Mutex::new(VecDeque::from([step])),
            attempts: AtomicU64::new(0),
        };
        let run = drive(&config, &session, &script);
        tokio::pin!(run);
        tokio::select! { ()=&mut run => panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
        let status = *session.connection.borrow();
        assert!(
            matches!(status.state,ConnectionState::Backoff { retry_at } if retry_at==tokio::time::Instant::now()+Duration::from_millis(500))
        );
        assert!(status.generation > old.generation);
        assert!(old.cancel.is_cancelled());
        assert_eq!(script.attempts.load(Ordering::Relaxed), 1);
        tokio::time::advance(Duration::from_millis(499)).await;
        tokio::select! { ()=&mut run=>panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
        assert_eq!(script.attempts.load(Ordering::Relaxed), 1);
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::select! { ()=&mut run=>panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
        assert_eq!(script.attempts.load(Ordering::Relaxed), 2);
        assert_eq!(session.connection.borrow().state, ConnectionState::Online);
        session.root.cancel();
        run.await;
    }
}
#[tokio::test(start_paused = true)]
async fn removal_cancels_backoff_without_another_connection() {
    let session = session();
    let config = ClientConfig {
        jitter: Arc::new(ZeroJitter),
        ..ClientConfig::default()
    };
    let script = Script {
        steps: Mutex::new(VecDeque::from([Err(ClientError::Gateway(
            papo_api::gateway::Error::HeartbeatTimeout,
        ))])),
        attempts: AtomicU64::new(0),
    };
    let run = drive(&config, &session, &script);
    tokio::pin!(run);
    tokio::select! { ()=&mut run=>panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
    session.root.cancel();
    run.await;
    tokio::time::advance(Duration::from_secs(100)).await;
    assert_eq!(script.attempts.load(Ordering::Relaxed), 1);
}

struct RateLimited;
impl Connector for RateLimited {
    fn connect<'a>(&'a self, _: &'a SessionInner, _: &'a Epoch) -> IoFuture<'a, Box<dyn Socket>> {
        Box::pin(async { Err(ClientError::Api(papo_api::Error::Http { status: 429 })) })
    }
}
#[tokio::test(start_paused = true)]
async fn rate_limited_identity_probe_backs_off_without_expiring_account() {
    let session = session();
    let old = session.epochs.current().generation;
    let config = ClientConfig {
        jitter: Arc::new(ZeroJitter),
        ..ClientConfig::default()
    };
    let run = drive(&config, &session, &RateLimited);
    tokio::pin!(run);
    tokio::select! { ()=&mut run=>panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
    assert!(matches!(
        session.connection.borrow().state,
        ConnectionState::Backoff { .. }
    ));
    assert_eq!(*session.auth.borrow(), AuthState::NeedsServer);
    assert!(session.epochs.current().generation > old);
    session.root.cancel();
    run.await;
}

struct PrivateServer;
impl Connector for PrivateServer {
    fn connect<'a>(&'a self, _: &'a SessionInner, _: &'a Epoch) -> IoFuture<'a, Box<dyn Socket>> {
        Box::pin(async {
            Err(ClientError::Api(papo_api::Error::Problem {
                status: 403,
                problem: Box::new(papo_api::wire::Problem {
                    r#type: "/errors/server-access-required".into(),
                    ..Default::default()
                }),
            }))
        })
    }
}
#[tokio::test(start_paused = true)]
async fn background_private_server_gate_is_published_in_the_auth_observable() {
    let session = session();
    let config = ClientConfig {
        jitter: Arc::new(ZeroJitter),
        ..ClientConfig::default()
    };
    let run = drive(&config, &session, &PrivateServer);
    tokio::pin!(run);
    tokio::select! { ()=&mut run=>panic!("driver stopped"), ()=tokio::task::yield_now()=>{} }
    assert_eq!(*session.auth.borrow(), AuthState::NeedsServerPassword);
    assert!(matches!(
        session.connection.borrow().state,
        ConnectionState::Backoff { .. }
    ));
    session.root.cancel();
    run.await;
}
