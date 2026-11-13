//! Deterministic denied-response, read-pin and account-reference races without network or clock sleeps.

use super::*;
use crate::{
    AuthState, Client, ClientConfig, Session, Uuid,
    secret::{CredentialGeneration, SecretKey, test_store::ScriptedSecretStore},
};
use std::{
    future::Future,
    sync::{Condvar, Mutex},
    task::Poll,
    time::{Duration, SystemTime},
};

fn client(directory: &tempfile::TempDir) -> Client {
    Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            disk: crate::DiskBudget {
                maintenance_interval: Duration::from_hours(1),
                ..crate::DiskBudget::default()
            },
            ..ClientConfig::default()
        },
    )
    .unwrap()
}
async fn session(client: &Client, user: u128) -> Session {
    let handle = client.handle();
    let address = crate::storage_tests::address(1);
    let account = crate::AccountId::new(address.id(), Uuid::from_u128(user));
    let inner = crate::session::SessionInner::new(
        account,
        address,
        SecretKey::new(account, CredentialGeneration::new(1)),
        Arc::clone(&handle.inner.pool.next),
        handle.inner.pool.root.child_token(),
        AuthState::Ready,
    );
    handle.inner.pool.insert(Arc::clone(&inner)).await;
    Session {
        inner,
        client: handle,
    }
}
async fn grant(session: &Session, source: MediaSource, bytes: &[u8]) -> [u8; 32] {
    let store = &session.client.inner.media;
    let _gate = store.gate.lock().await;
    let digest = store.install(Arc::from(bytes)).await.unwrap();
    store
        .grant(
            session.clone(),
            session.inner.epochs.current().generation,
            source,
            index::Reference {
                digest,
                content_type: None,
                etag: None,
                modified: None,
                range: None,
            },
        )
        .await
        .unwrap();
    digest
}

#[test]
fn denial_queued_on_media_gate_cannot_revoke_a_reconnected_generations_grant() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = client(&directory);
    client
        .block_on(async {
            let session = session(&client, 1).await;
            let source = MediaSource::Attachment(Uuid::from_u128(1));
            grant(&session, source.clone(), b"old encoded fixture").await;
            let store = &session.client.inner.media;
            let old = session.inner.epochs.current();
            let gate = store.gate.lock().await;
            let denied = store.revoke_denied(&session, old.generation, &source);
            tokio::pin!(denied);
            std::future::poll_fn(|cx| {
                assert!(denied.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            let new = session.inner.epochs.advance().unwrap();
            let digest = store
                .install(Arc::from(b"new encoded fixture".as_slice()))
                .await
                .unwrap();
            store
                .grant(
                    session.clone(),
                    new.generation,
                    source.clone(),
                    index::Reference {
                        digest,
                        content_type: None,
                        etag: None,
                        modified: None,
                        range: None,
                    },
                )
                .await
                .unwrap();
            drop(gate);
            assert!(matches!(
                denied.await,
                Err(MediaError::Session(ClientError::Superseded))
            ));
            // A callback already admitted before reconnect must also reject on the writer thread.
            assert!(matches!(
                store.revoke_family(&session, old.generation, &source).await,
                Err(MediaError::Session(ClientError::Superseded))
            ));
            assert_eq!(
                session
                    .media()
                    .cached(source.clone())
                    .await
                    .unwrap()
                    .value
                    .bytes
                    .as_ref(),
                b"new encoded fixture"
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

struct HeldRead(Arc<(Mutex<bool>, Condvar)>);
impl Drop for HeldRead {
    fn drop(&mut self) {
        let (lock, ready) = &*self.0;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    }
}

#[test]
fn held_read_allows_other_account_hits_and_survives_eviction_collection_and_ref_removal() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = client(&directory);
    client
        .block_on(async {
            let first = session(&client, 1).await;
            let other = session(&client, 2).await;
            let source = MediaSource::Attachment(Uuid::from_u128(1));
            let other_source = MediaSource::Attachment(Uuid::from_u128(2));
            let digest = grant(&first, source.clone(), b"held encoded fixture").await;
            grant(&other, other_source.clone(), b"independent encoded fixture").await;
            let store = &first.client.inner.media;
            let path = store.root.media_content(&digest);
            let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(now))
                .unwrap();
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = HeldRead(Arc::new((Mutex::new(false), Condvar::new())));
            let probe_release = Arc::clone(&release.0);
            let probe_entered = Arc::clone(&entered);
            *store.read_probe.lock().unwrap() = Some(Arc::new(move |reading| {
                if reading == digest {
                    probe_entered.notify_one();
                    let (lock, ready) = &*probe_release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = ready.wait(released).unwrap();
                    }
                }
            }));
            let media = first.media();
            let reading = tokio::spawn(async move { media.cached(source).await });
            entered.notified().await;
            assert!(
                store.gate.try_lock().is_ok(),
                "file hashing must not retain global exclusion"
            );
            assert_eq!(
                other
                    .media()
                    .cached(other_source.clone())
                    .await
                    .unwrap()
                    .value
                    .bytes
                    .as_ref(),
                b"independent encoded fixture"
            );
            assert!(store.evict_one().await.unwrap().is_some());
            assert!(path.exists());
            assert!(matches!(
                other.media().cached(other_source).await,
                Err(MediaError::Unavailable)
            ));
            store.clear_refs(first.account()).await.unwrap();
            store.collect_step(Duration::ZERO, now).await.unwrap();
            assert!(path.exists());
            drop(release);
            assert!(matches!(
                reading.await.unwrap(),
                Err(MediaError::Unavailable)
            ));
            while !store.collect_step(Duration::ZERO, now).await.unwrap() {}
            assert!(!path.exists());
        })
        .unwrap();
    client.shutdown().unwrap();
}
