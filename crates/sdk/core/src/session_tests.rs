//! Account handle revocation, retirement fences and credential cancellation without network or real time.

use crate::{
    AccountId, AuthService, AuthState, ClientError, Generated, ServerAddress, SessionEventError,
    SessionEvents,
    secret::{
        CredentialGeneration, CredentialState, SecretError, SecretKey, SecretStoreBoundary,
        SecretValue,
        test_store::{Behavior, Operation, ScriptedSecretStore},
    },
    session::SessionInner,
    session_pool::SessionPool,
};
use std::sync::{Arc, atomic::AtomicU64};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn session() -> Arc<SessionInner> {
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), papo_api::Uuid::nil());
    SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::new(AtomicU64::new(1)),
        CancellationToken::new(),
        AuthState::Ready,
    )
}
#[tokio::test(start_paused = true)]
async fn queued_old_event_is_dropped_and_removal_closes_subscriptions() {
    let inner = session();
    let mut events = SessionEvents {
        receiver: inner.events.subscribe(),
        inner: Arc::clone(&inner),
    };
    let old = inner.epochs.current();
    inner
        .events
        .send(Generated {
            generation: old.generation,
            value: papo_api::gateway::Event::decode(br#"{"type":"heartbeat_ack"}"#).unwrap(),
        })
        .ok()
        .unwrap();
    let current = inner.epochs.advance().unwrap();
    inner
        .events
        .send(Generated {
            generation: current.generation,
            value: papo_api::gateway::Event::decode(br#"{"type":"heartbeat_ack"}"#).unwrap(),
        })
        .ok()
        .unwrap();
    assert_eq!(events.next().await.unwrap().generation, current.generation);
    let mut auth = inner.auth.subscribe();
    let mut connection = inner.connection.subscribe();
    auth.borrow_and_update();
    connection.borrow_and_update();
    inner.stop().await;
    auth.borrow_and_update();
    connection.borrow_and_update();
    assert!(auth.changed().await.is_err());
    assert!(connection.changed().await.is_err());
    assert!(matches!(
        events.next().await,
        Err(SessionEventError::Closed)
    ));
}
#[tokio::test(start_paused = true)]
async fn session_stop_waits_for_credential_user_and_preserves_uncertainty() {
    let inner = session();
    let store = Arc::new(ScriptedSecretStore::default());
    let secrets = Arc::new(SecretStoreBoundary::new(
        store.clone(),
        Duration::from_secs(60),
    ));
    secrets
        .activate(inner.account, inner.key.generation())
        .unwrap();
    store.push(
        Operation::Write,
        Behavior::Detached(Duration::from_secs(10)),
    );
    let token = inner.tasks.token();
    let task_inner = Arc::clone(&inner);
    let task_secrets = Arc::clone(&secrets);
    let task = tokio::spawn(async move {
        let _token = token;
        task_secrets
            .write(task_inner.key, SecretValue::new(vec![]), &task_inner.root)
            .await
    });
    while store.calls(Operation::Write) == 0 {
        tokio::task::yield_now().await;
    }
    inner.stop().await;
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        secrets.state(inner.key).unwrap(),
        CredentialState::Uncertain
    );
    let cleanup = CancellationToken::new();
    assert!(matches!(
        secrets.delete(inner.key, &cleanup).await,
        Err(SecretError::Uncertain(_))
    ));
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    secrets.delete(inner.key, &cleanup).await.unwrap();
    assert!(store.snapshot(inner.key).is_none());
}
#[tokio::test(start_paused = true)]
async fn account_and_server_retirement_reject_late_login_without_blocking_other_accounts() {
    let pool = SessionPool::new();
    let inner = session();
    let account = inner.account;
    let other = AccountId::new(account.server(), papo_api::Uuid::from_u128(1));
    let admitted = pool.admit_login();
    assert!(pool.login_is_current(account, admitted));
    pool.stop(account).await;
    assert!(!pool.login_is_current(account, admitted));
    assert!(pool.login_is_current(other, admitted));
    let fresh = pool.admit_login();
    assert!(pool.login_is_current(account, fresh));
    pool.retire_server(account.server());
    assert!(!pool.login_is_current(other, fresh));
}
#[test]
fn auth_failure_states_distinguish_transport_expiry_and_private_server_grant() {
    assert_eq!(
        AuthService::failure_state(
            &ClientError::Gateway(papo_api::gateway::Error::HeartbeatTimeout),
            true
        ),
        AuthState::NeedsServer
    );
    assert_eq!(
        AuthService::failure_state(
            &ClientError::Api(papo_api::Error::Http { status: 401 }),
            true
        ),
        AuthState::Expired
    );
    assert_eq!(
        AuthService::failure_state(
            &ClientError::Api(papo_api::Error::Http { status: 401 }),
            false
        ),
        AuthState::SignedOut
    );
    let problem = papo_api::wire::Problem {
        r#type: "/errors/server-access-required".into(),
        ..Default::default()
    };
    assert_eq!(
        AuthService::failure_state(
            &ClientError::Api(papo_api::Error::Problem {
                status: 403,
                problem: Box::new(problem)
            }),
            false
        ),
        AuthState::NeedsServerPassword
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_an_old_handle_never_stops_its_replacement() {
    let pool = SessionPool::new();
    let old = session();
    let replacement = session();
    pool.insert(Arc::clone(&replacement));
    pool.stop_if_current(&old).await;
    assert!(!replacement.root.is_cancelled());
    assert!(pool.get(replacement.account).is_some());
    pool.stop_if_current(&replacement).await;
    assert!(replacement.root.is_cancelled());
    assert!(pool.get(replacement.account).is_none());
}

#[test]
fn ordinary_operation_rejection_does_not_expire_authenticated_identity() {
    assert_eq!(
        AuthService::failure_state(
            &ClientError::Api(papo_api::Error::Http { status: 409 }),
            true
        ),
        AuthState::Ready
    );
}

#[test]
fn rate_limits_and_protocol_failures_do_not_expire_credentials() {
    for error in [
        papo_api::Error::NotDispatched,
        papo_api::Error::Http { status: 429 },
        papo_api::Error::Decode,
    ] {
        assert_eq!(
            AuthService::failure_state(&ClientError::Api(error), true),
            AuthState::NeedsServer
        );
    }
}

#[tokio::test(start_paused = true)]
async fn auth_registration_after_shutdown_cannot_leave_a_subscription_open() {
    let pool = SessionPool::new();
    pool.stop_all().await;
    let state = crate::observable::StateCell::new(AuthState::SignedOut);
    let mut subscription = state.subscribe();
    subscription.borrow_and_update();
    pool.register_auth(session().account.server(), &state);
    assert!(subscription.changed().await.is_err());
}
