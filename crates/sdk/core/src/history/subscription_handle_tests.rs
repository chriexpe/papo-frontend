//! Public history subscription and stamp validation without sockets; storage lifecycle has its own suite.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use papo_api::Uuid;
use tokio_util::sync::CancellationToken;

use crate::{
    AccountId, AuthState, Client, ClientConfig, DataRoot, ServerAddress, Session,
    secret::{CredentialGeneration, SecretKey, test_store::ScriptedSecretStore},
    session::SessionInner,
};

use super::{HistoryClosedReason, HistoryError, HistoryHandle, HistoryUpdate, channel::Channel};

fn fixture() -> (tempfile::TempDir, Client, HistoryHandle) {
    let directory = tempfile::tempdir().unwrap();
    let client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let address = ServerAddress::parse("https://example.invalid").unwrap();
    let account = AccountId::new(address.id(), Uuid::nil());
    let inner = SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::new(AtomicU64::new(1)),
        CancellationToken::new(),
        AuthState::Ready,
    );
    let session = Session {
        inner,
        client: client.handle(),
    };
    let channel = Arc::new(Channel::new(
        Uuid::nil(),
        session.inner.history.changes.clone(),
        Arc::clone(&session.inner.history.counters),
    ));
    client
        .block_on(async {
            let state = channel.state.lock().await;
            channel
                .publish(&state, session.inner.epochs.current().generation)
                .unwrap();
            session
                .inner
                .history
                .channels
                .lock()
                .await
                .insert(Uuid::nil(), Arc::clone(&channel));
        })
        .unwrap();
    channel.opens.fetch_add(1, Ordering::Relaxed);
    (directory, client, HistoryHandle { session, channel })
}

async fn visible_snapshot(client: &Client, handle: &HistoryHandle) -> Arc<super::HistorySnapshot> {
    let account = handle.account();
    client
        .handle()
        .inner
        .registry
        .writer
        .write(move |tx| {
            tx.execute(
                "INSERT INTO servers VALUES(?1,'https://example.invalid',0,0,0)",
                [account.server().as_bytes()],
            )?;
            tx.execute(
                "INSERT INTO accounts(server_id,user_id,position,created_at_us) VALUES(?1,?2,0,0)",
                rusqlite::params![account.server().as_bytes(), account.user().as_bytes()],
            )?;
            Ok::<_, crate::StorageError>(())
        })
        .await
        .unwrap();
    let generation = handle.session.generation();
    let mut state = handle.channel.state.lock().await;
    let message = papo_api::wire::Message {
        id: Some(Uuid::from_u128(7).to_string()),
        channel_id: Some(handle.channel().to_string()),
        created_at: Some("2026-01-01T00:00:00.123456Z".into()),
        ..papo_api::wire::Message::default()
    };
    let revision = state.model.next_revision().unwrap();
    state
        .model
        .live_message(&message, generation.get(), revision)
        .unwrap();
    handle.channel.publish(&state, generation).unwrap();
    drop(state);
    let snapshot = handle.subscribe().await.unwrap().snapshot;
    handle
        .validate_snapshot(generation, snapshot.version)
        .await
        .unwrap();
    snapshot
}

async fn block_full_writer(
    client: &Client,
) -> (
    std::sync::mpsc::Sender<()>,
    tokio::task::JoinHandle<Result<(), crate::StorageError>>,
) {
    let writer = client.handle().inner.registry.writer.clone();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        writer
            .write(move |_| {
                let _ = entered.send(());
                blocked
                    .recv()
                    .map_err(|_| crate::StorageError::InvalidState)?;
                Ok::<_, crate::StorageError>(())
            })
            .await
    });
    waiting.await.unwrap();
    (release, task)
}

#[test]
fn queued_visible_report_rejects_a_newer_same_generation_publication_before_sql_mutation() {
    let (_directory, mut client, handle) = fixture();
    client
        .block_on(async {
            let snapshot = visible_snapshot(&client, &handle).await;
            let point = crate::ReadPoint::message(&snapshot.messages[0]).unwrap();
            let registry = client.handle().inner.registry.clone();
            let (release, blocked) = block_full_writer(&client).await;
            let mut report = Box::pin(registry.advance_visible_read(
                handle.clone(),
                snapshot.generation,
                snapshot.version,
                point,
            ));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(report.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let state = handle.channel.state.lock().await;
            handle.channel.publish(&state, snapshot.generation).unwrap();
            drop(state);
            release.send(()).unwrap();
            blocked.await.unwrap().unwrap();
            assert!(matches!(
                report.await,
                Err(crate::ReadingError::History(HistoryError::StaleSnapshot))
            ));
            assert!(
                registry
                    .load_read_points(handle.account())
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(handle.session.reading_state(handle.channel()).is_none());
            handle.session.inner.stop().await;
        })
        .unwrap();
    drop(handle);
    client.shutdown().unwrap();
}

#[test]
fn queued_visible_report_rejects_revocation_of_its_original_channel_before_sql_mutation() {
    let (_directory, mut client, handle) = fixture();
    client
        .block_on(async {
            let snapshot = visible_snapshot(&client, &handle).await;
            let point = crate::ReadPoint::message(&snapshot.messages[0]).unwrap();
            let registry = client.handle().inner.registry.clone();
            let (release, blocked) = block_full_writer(&client).await;
            let mut report = Box::pin(registry.advance_visible_read(
                handle.clone(),
                snapshot.generation,
                snapshot.version,
                point,
            ));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(report.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            handle.channel.close(HistoryClosedReason::Forbidden);
            let replacement = Arc::new(Channel::new(
                handle.channel(),
                handle.session.inner.history.changes.clone(),
                Arc::clone(&handle.session.inner.history.counters),
            ));
            {
                let state = replacement.state.lock().await;
                replacement.publish(&state, snapshot.generation).unwrap();
            }
            handle
                .session
                .inner
                .history
                .channels
                .lock()
                .await
                .insert(handle.channel(), replacement);
            release.send(()).unwrap();
            blocked.await.unwrap().unwrap();
            assert!(matches!(
                report.await,
                Err(crate::ReadingError::History(HistoryError::Closed(
                    HistoryClosedReason::Forbidden
                )))
            ));
            assert!(
                registry
                    .load_read_points(handle.account())
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(handle.session.reading_state(handle.channel()).is_none());
            handle.session.inner.stop().await;
        })
        .unwrap();
    drop(handle);
    client.shutdown().unwrap();
}

#[test]
fn queued_visible_report_rejects_a_replaced_account_generation_before_sql_mutation() {
    let (_directory, mut client, handle) = fixture();
    client
        .block_on(async {
            let snapshot = visible_snapshot(&client, &handle).await;
            let point = crate::ReadPoint::message(&snapshot.messages[0]).unwrap();
            let registry = client.handle().inner.registry.clone();
            let (release, blocked) = block_full_writer(&client).await;
            let mut report = Box::pin(registry.advance_visible_read(
                handle.clone(),
                snapshot.generation,
                snapshot.version,
                point,
            ));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(report.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            handle.session.inner.epochs.advance().unwrap();
            release.send(()).unwrap();
            blocked.await.unwrap().unwrap();
            assert!(matches!(
                report.await,
                Err(crate::ReadingError::History(HistoryError::Session(
                    crate::ClientError::Superseded
                )))
            ));
            assert!(
                registry
                    .load_read_points(handle.account())
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(handle.session.reading_state(handle.channel()).is_none());
            handle.session.inner.stop().await;
        })
        .unwrap();
    drop(handle);
    client.shutdown().unwrap();
}

#[test]
fn public_subscription_validates_current_stamp_and_rejects_old_version_and_epoch() {
    let (_directory, mut client, handle) = fixture();
    client
        .block_on(async {
            let before = handle.subscribe().await.unwrap();
            let same = handle
                .validate_snapshot(before.snapshot.generation, before.snapshot.version)
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&same, &before.snapshot));
            let state = handle.channel.state.lock().await;
            handle
                .channel
                .publish(&state, before.snapshot.generation)
                .unwrap();
            drop(state);
            assert!(matches!(
                handle
                    .validate_snapshot(before.snapshot.generation, before.snapshot.version)
                    .await,
                Err(HistoryError::StaleSnapshot)
            ));
            let epoch = handle.session.inner.epochs.advance().unwrap();
            handle
                .session
                .inner
                .history
                .generation_changed(epoch.generation)
                .await
                .unwrap();
            assert!(matches!(
                handle
                    .validate_snapshot(before.snapshot.generation, before.snapshot.version)
                    .await,
                Err(HistoryError::Session(crate::ClientError::Superseded))
            ));
            let current = handle.subscribe().await.unwrap();
            assert_eq!(current.snapshot.generation, epoch.generation);
            assert!(current.snapshot.version > before.snapshot.version);
            handle.session.inner.stop().await;
        })
        .unwrap();
    drop(handle);
    client.shutdown().unwrap();
}

#[test]
fn public_subscription_receives_idle_session_stop_and_keeps_its_first_close_reason() {
    let (_directory, mut client, handle) = fixture();
    client
        .block_on(async {
            let mut subscription = handle.subscribe().await.unwrap();
            handle.channel.close(HistoryClosedReason::Forbidden);
            handle.session.inner.stop().await;
            subscription.updates.changed().await.unwrap();
            assert!(matches!(
                *subscription.updates.borrow_and_update(),
                HistoryUpdate::Closed(HistoryClosedReason::Forbidden)
            ));
            assert!(matches!(
                handle.subscribe().await,
                Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
            ));
        })
        .unwrap();
    drop(handle);
    client.shutdown().unwrap();
}
