//! Credential uncertainty remains a durable removal failure until fenced read-back.

use crate::secret::{
    SecretKey, SecretValue,
    test_store::{Behavior, Operation},
};
use crate::storage_tests::{Fixture, account, address};
use crate::{
    UnixMicros,
    removal::{RemovalFailureKind, RemovalStep},
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[test]
fn late_secret_delete_keeps_removal_pending_until_read_back_confirms_absence() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        tokio::time::pause();
        let server = address(1);
        let id = account(&server, 1);
        let registry = fixture.storage().registry();
        let jobs = fixture.storage().removals();
        registry
            .add_server(server.clone(), UnixMicros(1))
            .await
            .unwrap();
        let generation = registry
            .add_account(id, None, None, true, UnixMicros(1))
            .await
            .unwrap();
        let key = SecretKey::new(id, generation);
        jobs.secrets.activate(id, generation).unwrap();
        jobs.secrets
            .write(key, SecretValue::new(Vec::new()), &CancellationToken::new())
            .await
            .unwrap();
        fixture.secrets.push(
            Operation::Delete,
            Behavior::Detached(Duration::from_secs(2)),
        );
        jobs.begin_account(id).await.unwrap();
        let awake = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let attempt = tokio::spawn({
            let jobs = jobs.clone();
            async move { jobs.resume().await.unwrap() }
        });
        while fixture.secrets.calls(Operation::Delete) == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(1100)).await;
        let pending = attempt.await.unwrap();
        assert_eq!(
            (pending[0].step, pending[0].failure),
            (
                RemovalStep::Secrets,
                Some(RemovalFailureKind::SecretUncertain)
            )
        );
        assert!(registry.accounts(server.id()).await.unwrap().is_empty());
        assert!(fixture.secrets.snapshot(key).is_some());
        // An unfenced observation cannot acknowledge cleanup while the old delete runs.
        assert_eq!(
            jobs.resume().await.unwrap()[0].failure,
            Some(RemovalFailureKind::SecretUncertain)
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert!(jobs.resume().await.unwrap().is_empty());
        assert!(fixture.secrets.snapshot(key).is_none());
        assert_eq!(fixture.secrets.calls(Operation::Delete), 1);
        assert!(fixture.secrets.calls(Operation::Read) >= 3);
        awake.abort();
    });
}
