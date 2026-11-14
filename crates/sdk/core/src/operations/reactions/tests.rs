//! Deterministic pending-command grouping; real FULL enqueue acknowledgement is tested over loopback.

use super::*;

type AdmissionReply = oneshot::Receiver<Result<ReactionAdmission, ReactionError>>;

fn command(message: u128, generation: u64, local: u128, add: bool) -> (Command, AdmissionReply) {
    let (reply, receiver) = oneshot::channel();
    (
        Command {
            generation: Generation(generation),
            intent: ReactionIntent {
                channel: Uuid::from_u128(1),
                message: MessageTarget::Remote(Uuid::from_u128(message)),
                emoji: ReactionEmoji::Unicode("👍".into()),
                add,
                local: Uuid::from_u128(local),
                at: UnixMicros(0),
            },
            reply,
        },
        receiver,
    )
}

#[test]
fn pending_add_remove_add_retains_the_latest_desired_membership_and_identity() {
    let (first, _first) = command(10, 1, 20, true);
    let (second, _second) = command(10, 1, 21, false);
    let (third, _third) = command(10, 1, 22, true);
    let mut pending = VecDeque::from([first, second, third]);
    let group = adjacent_group(&mut pending).unwrap();
    assert_eq!(group.len(), 3);
    assert!(pending.is_empty());
    let final_intent = &group.last().unwrap().intent;
    assert!(final_intent.add);
    assert_eq!(final_intent.local, Uuid::from_u128(22));
}

#[test]
fn an_intervening_message_preserves_channel_command_order() {
    let (first, _first) = command(10, 1, 20, true);
    let (other, _other) = command(11, 1, 21, true);
    let (third, _third) = command(10, 1, 22, false);
    let mut pending = VecDeque::from([first, other, third]);
    for local in [20, 21, 22] {
        let group = adjacent_group(&mut pending).unwrap();
        assert_eq!(group.len(), 1);
        assert_eq!(group[0].intent.local, Uuid::from_u128(local));
    }
}

#[test]
fn reconnect_never_coalesces_an_old_command_into_a_new_generation() {
    let (first, _first) = command(10, 1, 20, true);
    let (second, _second) = command(10, 2, 21, false);
    let mut pending = VecDeque::from([first, second]);
    assert_eq!(adjacent_group(&mut pending).unwrap().len(), 1);
    assert_eq!(adjacent_group(&mut pending).unwrap().len(), 1);
}

fn offline_session() -> (tempfile::TempDir, crate::Client, Session) {
    use crate::{
        AccountId, AuthState, Client, ClientConfig, DataRoot, ServerAddress,
        secret::{SecretKey, test_store::ScriptedSecretStore},
        session::SessionInner,
    };
    use std::sync::atomic::AtomicU64;

    let directory = tempfile::tempdir().unwrap();
    let client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let handle = client.handle();
    let session = client
        .block_on(async {
            let address = ServerAddress::parse("https://example.invalid").unwrap();
            let account = AccountId::new(address.id(), Uuid::from_u128(1));
            handle
                .inner
                .registry
                .add_server(address.clone(), UnixMicros(0))
                .await
                .unwrap();
            let credential_generation = handle
                .inner
                .registry
                .add_account(account, None, None, true, UnixMicros(0))
                .await
                .unwrap();
            let inner = SessionInner::new(
                account,
                address,
                SecretKey::new(account, credential_generation),
                Arc::new(AtomicU64::new(1)),
                handle.inner.pool.root.child_token(),
                AuthState::Ready,
            );
            handle.inner.pool.insert(inner.clone()).await;
            Session {
                inner,
                client: handle.clone(),
            }
        })
        .unwrap();
    (directory, client, session)
}

#[test]
fn coalesced_waiters_receive_one_full_committed_final_intent_without_dispatch() {
    let (_directory, mut client, session) = offline_session();
    client
        .block_on(async {
            let epoch = session.generation().get();
            let (first, first_reply) = command(10, epoch, 20, true);
            let (second, second_reply) = command(10, epoch, 21, false);
            let (third, third_reply) = command(10, epoch, 22, true);
            let mut pending = VecDeque::from([first, second, third]);
            admit(&session, adjacent_group(&mut pending).unwrap()).await;
            assert_eq!(
                first_reply.await.unwrap().unwrap(),
                ReactionAdmission::CoalescedInto(Uuid::from_u128(22))
            );
            assert_eq!(
                second_reply.await.unwrap().unwrap(),
                ReactionAdmission::CoalescedInto(Uuid::from_u128(22))
            );
            assert_eq!(
                third_reply.await.unwrap().unwrap(),
                ReactionAdmission::Queued(Uuid::from_u128(22))
            );
            let snapshot = session
                .outgoing()
                .subscribe()
                .await
                .unwrap()
                .borrow()
                .clone();
            assert_eq!(snapshot.operations.len(), 1);
            assert_eq!(snapshot.operations[0].id, Uuid::from_u128(22));
            assert_eq!(snapshot.operations[0].state, crate::OperationState::Queued);
            assert_eq!(snapshot.operations[0].attempts, 0);
            assert!(matches!(
                snapshot.operations[0].kind.as_ref(),
                OperationKind::Reaction { add: true, .. }
            ));
        })
        .unwrap();
    let db = rusqlite::Connection::open(&client.handle().inner.registry.path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    client.shutdown().unwrap();
}

#[test]
fn stale_pending_reaction_is_not_persisted_and_accepted_intent_survives_epoch_change() {
    let (_directory, mut client, session) = offline_session();
    client
        .block_on(async {
            let old_generation = session.generation();
            let (old, old_reply) = command(10, old_generation.get(), 20, true);
            session.inner.epochs.advance().unwrap();
            admit(&session, vec![old]).await;
            assert!(matches!(
                old_reply.await.unwrap(),
                Err(ReactionError::Superseded)
            ));
            let snapshot = session
                .outgoing()
                .subscribe()
                .await
                .unwrap()
                .borrow()
                .clone();
            assert!(snapshot.operations.is_empty());
            let (current, current_reply) = command(10, session.generation().get(), 21, false);
            admit(&session, vec![current]).await;
            assert_eq!(
                current_reply.await.unwrap().unwrap(),
                ReactionAdmission::Queued(Uuid::from_u128(21))
            );
            // Advancing the same epoch boundary used by reconnect cannot revoke accepted durable intent.
            session.inner.epochs.advance().unwrap();
            let snapshot = session
                .outgoing()
                .subscribe()
                .await
                .unwrap()
                .borrow()
                .clone();
            assert_eq!(snapshot.operations.len(), 1);
            assert_eq!(snapshot.operations[0].id, Uuid::from_u128(21));
            assert_eq!(snapshot.operations[0].state, crate::OperationState::Queued);
            assert_eq!(snapshot.operations[0].attempts, 0);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn canceled_pending_callers_leave_no_durable_reaction() {
    let (_directory, mut client, session) = offline_session();
    client
        .block_on(async {
            let (command, reply) = command(10, session.generation().get(), 20, true);
            drop(reply);
            admit(&session, vec![command]).await;
            let snapshot = session
                .outgoing()
                .subscribe()
                .await
                .unwrap()
                .borrow()
                .clone();
            assert!(snapshot.operations.is_empty());
        })
        .unwrap();
    client.shutdown().unwrap();
}
