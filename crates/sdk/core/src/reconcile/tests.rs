//! In-memory journal bounds and generation fences; socket integration uses loopback tests.

use super::*;
use crate::{
    AccountId, AuthState, ServerAddress,
    secret::{CredentialGeneration, SecretKey},
};
use papo_api::gateway::{ChannelDeleteEvent, KnownEvent};
use std::{pin::Pin, sync::atomic::AtomicU64};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

type Next<'a> = Pin<Box<dyn Future<Output = Result<Option<Event>, ClientError>> + Send + 'a>>;
struct ScriptSocket {
    events: VecDeque<Event>,
    drained: Option<oneshot::Sender<()>>,
}
impl Socket for ScriptSocket {
    fn next(&mut self) -> Next<'_> {
        Box::pin(async {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            if let Some(drained) = self.drained.take() {
                let _ = drained.send(());
            }
            std::future::pending().await
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
        CancellationToken::new(),
        AuthState::Ready,
    )
}
fn event(id: u128) -> Event {
    Event::Known(KnownEvent::ChannelDelete(Box::new(ChannelDeleteEvent {
        channel_id: papo_api::Uuid::from_u128(id).to_string(),
    })))
}

#[tokio::test]
async fn events_are_journaled_in_socket_order_until_the_snapshot_finishes() {
    let session = session();
    let epoch = session.epochs.current();
    let (sender, receiver) = oneshot::channel();
    let mut socket = ScriptSocket {
        events: (1..=10).map(event).collect(),
        drained: Some(sender),
    };
    let mut journal = VecDeque::new();
    collect(&session, &epoch, &mut socket, &mut journal, async {
        receiver.await.unwrap();
        Ok(())
    })
    .await
    .unwrap();
    let ids: Vec<_> = journal
        .into_iter()
        .map(|event| {
            let Event::Known(KnownEvent::ChannelDelete(event)) = event else {
                panic!("unexpected event")
            };
            event.channel_id
        })
        .collect();
    assert_eq!(
        ids,
        (1..=10)
            .map(|id| papo_api::Uuid::from_u128(id).to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(session.resources.borrow().version, 0);
}

#[tokio::test]
async fn journal_overflow_fails_the_generation_without_growing_the_queue() {
    let session = session();
    let epoch = session.epochs.current();
    let mut socket = ScriptSocket {
        events: (0..=JOURNAL_CAPACITY).map(|id| event(id as u128)).collect(),
        drained: None,
    };
    let mut journal = VecDeque::new();
    let result = collect(
        &session,
        &epoch,
        &mut socket,
        &mut journal,
        std::future::pending::<Result<(), ClientError>>(),
    )
    .await;
    assert!(matches!(result, Err(ClientError::ReconcileOverflow)));
    assert_eq!(journal.len(), JOURNAL_CAPACITY);
    let work = session.history.counters.snapshot();
    assert_eq!(work.reconnect_journal_peak, JOURNAL_CAPACITY as u64);
    assert_eq!(work.reconnect_overflows, 1);
}

#[tokio::test]
async fn generation_replacement_discards_ready_snapshot_and_cancels_its_work() {
    let session = session();
    let old = session.epochs.current();
    session.epochs.advance().unwrap();
    let mut socket = ScriptSocket {
        events: VecDeque::new(),
        drained: None,
    };
    let mut journal = VecDeque::new();
    let result = collect(&session, &old, &mut socket, &mut journal, async {
        session.publish_resources(ResourceSnapshot::empty(old.generation))
    })
    .await;
    assert!(matches!(result, Err(ClientError::Superseded)));
    assert_eq!(session.resources.borrow().version, 0);
}

#[test]
fn publication_checks_epoch_and_version_as_one_operation() {
    let session = session();
    let old = session.epochs.current();
    session
        .publish_resources(ResourceSnapshot::empty(old.generation))
        .unwrap();
    let new = session.epochs.advance().unwrap();
    assert!(matches!(
        session.publish_resources(ResourceSnapshot::empty(old.generation)),
        Err(ClientError::Superseded)
    ));
    session
        .publish_resources(ResourceSnapshot::empty(new.generation))
        .unwrap();
    let snapshot = session.resources.borrow();
    assert_eq!(snapshot.version, 2);
    assert_eq!(snapshot.generation, new.generation);
}
