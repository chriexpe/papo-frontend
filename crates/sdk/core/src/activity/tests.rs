//! Pure monotonic activity transitions use an injected paused Tokio clock; no sockets or wall clock.
use super::*;
use papo_api::gateway::{PresenceUpdateEvent, TypingEvent};
fn event(typing: bool) -> Event {
    Event::Known(KnownEvent::Typing(Box::new(TypingEvent {
        channel_id: Uuid::from_u128(1).to_string(),
        user_id: Uuid::from_u128(2).to_string(),
        is_typing: typing,
    })))
}
#[tokio::test(start_paused = true)]
async fn typing_expires_at_ten_seconds_and_refresh_and_stop_are_exact() {
    let now = Instant::now();
    let mut state = ActivitySnapshot::empty(Generation(1));
    assert!(state.apply(&event(true), now));
    assert!(!state.expire(now + Duration::from_secs(9)));
    state.apply(&event(true), now + Duration::from_secs(9));
    assert!(!state.expire(now + Duration::from_secs(10)));
    assert!(state.expire(now + Duration::from_secs(19)));
    assert!(state.typing.is_empty());
    state.apply(&event(true), now);
    state.apply(&event(false), now);
    assert!(state.typing.is_empty());
}
#[tokio::test(start_paused = true)]
async fn offline_presence_retires_typing_and_entry_capacity_is_bounded() {
    let now = Instant::now();
    let mut state = ActivitySnapshot::empty(Generation(1));
    state.apply(&event(true), now);
    state.apply(
        &Event::Known(KnownEvent::PresenceUpdate(Box::new(PresenceUpdateEvent {
            user_id: Uuid::from_u128(2).to_string(),
            status: "offline".into(),
            ..PresenceUpdateEvent::default()
        }))),
        now,
    );
    assert!(state.typing.is_empty());
    for user in 1..=1025 {
        state.apply(
            &Event::Known(KnownEvent::Typing(Box::new(TypingEvent {
                channel_id: Uuid::from_u128(1).to_string(),
                user_id: Uuid::from_u128(user).to_string(),
                is_typing: true,
            }))),
            now,
        );
    }
    assert_eq!(state.typing.len(), 1024);
    assert!(state.expire(now + Duration::from_secs(10)));
    assert!(state.typing.is_empty());
}

fn session() -> Arc<SessionInner> {
    use crate::{
        AccountId, AuthState, ServerAddress,
        secret::{CredentialGeneration, SecretKey},
    };
    use std::sync::atomic::AtomicU64;
    use tokio_util::sync::CancellationToken;
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), Uuid::from_u128(1));
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
async fn expiry_worker_publishes_without_new_gateway_events_and_revocation_closes() {
    let session = session();
    let worker = tokio::spawn(expire_loop(session.clone()));
    let mut source = session.activity.subscribe();
    receive(&session, session.epochs.current().generation, &event(true));
    assert_eq!(source.borrow_and_update().typing.len(), 1);
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(9)).await;
    assert!(!source.borrow().typing.is_empty());
    tokio::time::advance(Duration::from_secs(1)).await;
    source.changed().await.unwrap();
    assert!(source.borrow_and_update().typing.is_empty());
    session.stop().await;
    worker.await.unwrap();
    source.borrow_and_update();
    assert!(source.changed().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn connection_loss_clears_live_state_and_old_generation_cannot_resurrect_it() {
    let session = session();
    let old = session.epochs.current().generation;
    receive(&session, old, &event(true));
    receive(
        &session,
        old,
        &Event::Known(KnownEvent::PresenceUpdate(Box::new(PresenceUpdateEvent {
            user_id: Uuid::from_u128(2).to_string(),
            status: "online".into(),
            ..PresenceUpdateEvent::default()
        }))),
    );
    assert_eq!(session.activity.borrow().presence.len(), 1);
    let next = session.epochs.advance().unwrap().generation;
    session
        .status(
            next,
            ConnectionState::Backoff {
                retry_at: Instant::now() + Duration::from_secs(1),
            },
        )
        .unwrap();
    receive(&session, old, &event(true));
    assert!(session.activity.borrow().typing.is_empty());
    assert!(session.activity.borrow().presence.is_empty());
    assert_eq!(session.activity.borrow().generation, next);
    receive(&session, next, &event(true));
    assert_eq!(session.activity.borrow().typing.len(), 1);
    session.status(next, ConnectionState::Offline).unwrap();
    assert!(session.activity.borrow().typing.is_empty());
    session.stop().await;
}
