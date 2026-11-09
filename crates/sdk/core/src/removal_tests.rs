//! Removal recovery and isolation; session/media ownership is represented by test hooks.

use crate::removal::{RemovalFailureKind, RemovalStep};
use crate::secret::{SecretKey, SecretStoreBoundary, SecretValue};
use crate::storage_tests::{Fixture, account, address};
use crate::{StorageError, UnixMicros};
use std::sync::atomic::Ordering;
use tokio_util::sync::CancellationToken;

#[test]
fn account_removal_is_hidden_durably_and_resumed_before_open_returns() {
    let mut fixture = Fixture::new();
    let server = address(1);
    let first = account(&server, 1);
    let other = account(&server, 2);
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry
            .add_server(server.clone(), UnixMicros(1))
            .await
            .unwrap();
        registry
            .add_account(first, None, None, true, UnixMicros(1))
            .await
            .unwrap();
        registry
            .add_account(other, None, None, true, UnixMicros(1))
            .await
            .unwrap();
        fixture
            .storage()
            .removals()
            .begin_account(first)
            .await
            .unwrap();
        assert_eq!(registry.accounts(server.id()).await.unwrap()[0].id, other);
    });
    assert_eq!(fixture.hooks.stops.load(Ordering::SeqCst), 0);
    fixture.reopen();
    fixture.runtime().block_on(async {
        assert!(
            fixture
                .storage()
                .removals()
                .pending()
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fixture
                .storage()
                .registry()
                .accounts(server.id())
                .await
                .unwrap()
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            [other]
        );
    });
    assert_eq!(fixture.hooks.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn persistent_media_failure_retries_only_the_unfinished_step() {
    let mut fixture = Fixture::new();
    let server = address(1);
    let id = account(&server, 1);
    fixture.hooks.fail_media.store(true, Ordering::SeqCst);
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry
            .add_server(server.clone(), UnixMicros(1))
            .await
            .unwrap();
        registry
            .add_account(id, None, None, true, UnixMicros(1))
            .await
            .unwrap();
        fixture
            .storage()
            .removals()
            .begin_server(server.id())
            .await
            .unwrap();
        let pending = fixture.storage().removals().resume().await.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(
            (pending[0].step, pending[0].failure),
            (RemovalStep::MediaRefs, Some(RemovalFailureKind::MediaRefs))
        );
        assert!(registry.servers().await.unwrap().is_empty());
    });
    fixture.reopen();
    fixture.runtime().block_on(async {
        let pending = fixture.storage().removals().pending().await.unwrap();
        assert_eq!(pending[0].failure, Some(RemovalFailureKind::MediaRefs));
    });
    assert_eq!(fixture.hooks.stops.load(Ordering::SeqCst), 1);
    fixture.hooks.fail_media.store(false, Ordering::SeqCst);
    fixture.runtime().block_on(async {
        assert!(
            fixture
                .storage()
                .removals()
                .resume()
                .await
                .unwrap()
                .is_empty()
        );
    });
    assert_eq!(fixture.hooks.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn removal_cleans_only_its_account_and_generations_are_never_reused() {
    let fixture = Fixture::new();
    let server = address(1);
    let first = account(&server, 1);
    let other = account(&server, 2);
    let first_cache = fixture.root().account_cache(first);
    let other_cache = fixture.root().account_cache(other);
    std::fs::create_dir_all(first_cache.parent().unwrap()).unwrap();
    std::fs::write(&first_cache, []).unwrap();
    std::fs::write(&other_cache, []).unwrap();
    fixture.runtime().block_on(async {
        let registry=fixture.storage().registry();
        registry.add_server(server.clone(),UnixMicros(1)).await.unwrap();
        let old=registry.add_account(first,None,None,true,UnixMicros(1)).await.unwrap();
        registry.add_account(other,None,None,true,UnixMicros(1)).await.unwrap();
        registry.writer.write(move |tx|->Result<(),StorageError> {
            for id in [first,other] {
                tx.execute("INSERT INTO drafts(server_id,user_id,channel_id,text,mentions,updated_at_us) VALUES(?1,?2,zeroblob(16),'',x'',1)",rusqlite::params![id.server().as_bytes(),id.user().as_bytes()])?;
                tx.execute("INSERT INTO read_points(server_id,user_id,channel_id) VALUES(?1,?2,zeroblob(16))",rusqlite::params![id.server().as_bytes(),id.user().as_bytes()])?;
                tx.execute("INSERT INTO notification_ledger(server_id,user_id,message_id,delivered_at_us) VALUES(?1,?2,zeroblob(16),1)",rusqlite::params![id.server().as_bytes(),id.user().as_bytes()])?;
            }
            Ok(())
        }).await.unwrap();
        let boundary=SecretStoreBoundary::new(fixture.secrets.clone(),std::time::Duration::from_secs(1));
        boundary.activate(first,old).unwrap();
        boundary.write(SecretKey::new(first,old),SecretValue::new(Vec::new()),&CancellationToken::new()).await.unwrap();
        fixture.storage().removals().begin_account(first).await.unwrap();
        assert!(fixture.storage().removals().resume().await.unwrap().is_empty());
        assert!(fixture.secrets.snapshot(SecretKey::new(first,old)).is_none());
        let new=registry.add_account(first,None,None,false,UnixMicros(2)).await.unwrap();
        assert!(new.get()>old.get());
        let counts=registry.readers.query(registry.path.clone(),|tx| {
            ["drafts","read_points","notification_ledger"].into_iter().map(|table|tx.query_row(&format!("SELECT count(*) FROM {table}"),[],|row|row.get::<_,i64>(0))).collect::<Result<Vec<_>,_>>()
        }).await.unwrap();
        assert_eq!(counts,[1,1,1]);
    });
    assert!(!first_cache.exists());
    assert!(other_cache.exists());
}

#[test]
fn failed_stop_keeps_the_job_and_prevents_secret_or_file_deletion() {
    let fixture = Fixture::new();
    let server = address(1);
    let id = account(&server, 1);
    fixture.hooks.fail_stop.store(true, Ordering::SeqCst);
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry.add_server(server, UnixMicros(1)).await.unwrap();
        registry
            .add_account(id, None, None, true, UnixMicros(1))
            .await
            .unwrap();
        fixture
            .storage()
            .removals()
            .begin_account(id)
            .await
            .unwrap();
        let pending = fixture.storage().removals().resume().await.unwrap();
        assert_eq!(
            (pending[0].step, pending[0].failure),
            (
                RemovalStep::StopSession,
                Some(RemovalFailureKind::SessionStop)
            )
        );
        assert_eq!(fixture.hooks.media.load(Ordering::SeqCst), 0);
    });
}
