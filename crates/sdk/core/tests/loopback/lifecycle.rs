//! Exercises socket generations, revocation and persisted session restoration; transport decoding stays in API tests.

use papo_sdk::{ClientError, ConnectionState, Generated, SessionEventError, UnixMicros};
use papo_test_server::Config;

use super::fixture::{Fixture, Result, login, online, register, wait_for};

#[test]
fn socket_drop_reconnects_with_new_generation_and_filters_old_results_and_events() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "first-user").await?;
        let session = login(&auth, "first-user").await?;
        let first = online(&session).await?;
        let mut states = session.subscribe_connection();
        let mut events = session.subscribe_events();
        let old_result = session.auth().connected_devices().await?;
        assert_eq!(old_result.generation, first);
        fixture.server().disconnect_gateways();
        wait_for(&mut states, |status| {
            matches!(status.state, ConnectionState::Backoff { .. })
        })
        .await?;
        let reconnect = wait_for(&mut states, |status| {
            status.state == ConnectionState::Online && status.generation.get() > first.get()
        })
        .await?;
        assert_eq!(session.generation(), reconnect.generation);
        assert!(matches!(
            session.current(old_result),
            Err(ClientError::Superseded)
        ));
        let event = events.next().await?;
        assert_eq!(event.generation, reconnect.generation);
        assert!(session.current(event).is_ok());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn stale_logout_after_replacement_cannot_stop_the_current_account_session() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "replace-user").await?;
        let old = login(&auth, "replace-user").await?;
        let previous = online(&old).await?;
        let mut events = old.subscribe_events();
        let current = login(&handle.auth(server)?, "replace-user").await?;
        let generation = online(&current).await?;
        assert!(generation.get() > previous.get());
        assert!(old.is_closed());
        assert!(matches!(
            events.next().await,
            Err(SessionEventError::Closed)
        ));
        assert!(matches!(
            old.auth().logout().await,
            Err(ClientError::Closed)
        ));
        assert!(!current.is_closed());
        assert_eq!(handle.session(current.account())?.generation(), generation);
        assert_eq!(online(&current).await?, generation);
        assert!(current.auth().connected_devices().await.is_ok());
        assert!(matches!(
            old.current(Generated {
                generation: previous,
                value: ()
            }),
            Err(ClientError::Closed)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn explicit_shutdown_joins_and_reopen_restores_verified_credentials_without_login() -> Result {
    let mut fixture = Fixture::new(Config::default())?;
    let (account, old_session) = fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "restore-user").await?;
        let session = login(&auth, "restore-user").await?;
        online(&session).await?;
        Ok::<_, Box<dyn std::error::Error>>((session.account(), session))
    })??;
    let old_handle = fixture.client().handle();
    fixture.close_client()?;
    assert!(old_handle.is_closed());
    assert!(old_session.is_closed());
    drop(old_session);
    drop(old_handle);
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        online(&session).await?;
        assert_eq!(session.account(), account);
        assert_eq!(session.auth().state(), papo_sdk::AuthState::Ready);
        assert!(session.auth().connected_devices().await.is_ok());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
