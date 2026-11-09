//! Exercises account/server revocation and durable cleanup retry over sockets; storage crash tests stay beside storage.

use papo_sdk::{
    ClientError, SessionEventError, UnixMicros,
    removal::{RemovalFailureKind, RemovalStep, RemovalTarget},
    secret::{
        SecretKey, SecretStoreError,
        test_store::{Behavior, Operation},
    },
};
use papo_test_server::Config;

use super::fixture::{Fixture, Result, login, online, register};

#[test]
fn account_removal_revokes_only_its_handles_and_server_removal_revokes_the_rest() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "remove-one").await?;
        register(&auth, "remove-two").await?;
        let first = login(&auth, "remove-one").await?;
        let second = login(&handle.auth(server)?, "remove-two").await?;
        online(&first).await?;
        let second_generation = online(&second).await?;
        let mut first_events = first.subscribe_events();
        let mut second_events = second.subscribe_events();
        let saved = handle
            .subscribe_registry()
            .borrow()
            .accounts
            .iter()
            .find(|saved| saved.id == first.account())
            .cloned()
            .ok_or("saved account missing")?;
        let key = SecretKey::new(saved.id, saved.credential_generation);
        assert!(fixture.store.snapshot(key).is_some());
        handle.remove_account(first.account()).await?;
        assert!(first.is_closed());
        assert!(matches!(
            first_events.next().await,
            Err(SessionEventError::Closed)
        ));
        assert!(matches!(
            handle.session(first.account()),
            Err(ClientError::NotFound)
        ));
        assert!(fixture.store.snapshot(key).is_none());
        assert!(!second.is_closed());
        assert_eq!(online(&second).await?, second_generation);
        assert_eq!(handle.subscribe_registry().borrow().accounts.len(), 1);
        assert_eq!(handle.subscribe_registry().borrow().removals.len(), 0);
        handle.remove_server(server).await?;
        assert!(second.is_closed());
        assert!(matches!(
            second_events.next().await,
            Err(SessionEventError::Closed)
        ));
        let snapshot = handle.subscribe_registry().borrow().clone();
        assert_eq!(snapshot.servers.len(), 0);
        assert_eq!(snapshot.accounts.len(), 0);
        assert_eq!(snapshot.removals.len(), 0);
        assert!(matches!(handle.auth(server), Err(ClientError::NotFound)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn failed_secret_cleanup_stays_hidden_and_observable_across_retry_and_restart() -> Result {
    let mut fixture = Fixture::new(Config::default())?;
    let (account, session) = fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "retry-remove").await?;
        let session = login(&auth, "retry-remove").await?;
        online(&session).await?;
        Ok::<_, Box<dyn std::error::Error>>((session.account(), session))
    })??;
    for _ in 0..3 {
        fixture.store.push(
            Operation::Delete,
            Behavior::Fail(SecretStoreError::Unavailable),
        );
    }
    fixture.run(async {
        let handle = fixture.client().handle();
        handle.remove_account(account).await?;
        assert!(session.is_closed());
        assert_pending_failure(&handle, account);
        handle.retry_removals().await?;
        assert_pending_failure(&handle, account);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    drop(session);
    fixture.reopen()?;
    fixture.run(async {
        let handle = fixture.client().handle();
        assert_pending_failure(&handle, account);
        assert!(matches!(
            handle.session(account),
            Err(ClientError::NotFound)
        ));
        handle.retry_removals().await?;
        let snapshot = handle.subscribe_registry().borrow().clone();
        assert_eq!(snapshot.accounts.len(), 0);
        assert_eq!(snapshot.removals.len(), 0);
        assert_eq!(snapshot.servers.len(), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

fn assert_pending_failure(handle: &papo_sdk::ClientHandle, account: papo_sdk::AccountId) {
    let snapshot = handle.subscribe_registry().borrow().clone();
    assert_eq!(snapshot.accounts.len(), 0);
    assert_eq!(snapshot.removals.len(), 1);
    let job = &snapshot.removals[0];
    assert_eq!(job.target, RemovalTarget::Account(account));
    assert_eq!(job.step, RemovalStep::Secrets);
    assert_eq!(job.failure, Some(RemovalFailureKind::SecretUncertain));
}
