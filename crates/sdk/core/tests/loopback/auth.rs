//! Verifies typed auth policy through loopback routes; credentials remain runtime-derived and unformatted.

use papo_api::wire::{
    DropConnectionRequest, LoginRequest, LoginServerRequest, PasswordResetConsumeRequest,
};
use papo_sdk::{AuthState, ClientError, ConnectionState, SessionEventError, UnixMicros};
use papo_test_server::{Config, Failure, RestStep};

use super::fixture::{Fixture, Result, login, online, password, register, wait_for};

#[test]
fn private_server_gate_login_refresh_and_logout_follow_public_auth_states() -> Result {
    let fixture = Fixture::new(Config {
        public: false,
        server_password: Some(password(2)),
        ..Config::default()
    })?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        assert!(register(&auth, "private-user").await.is_err());
        assert_eq!(auth.state(), AuthState::NeedsServerPassword);
        auth.server_login(LoginServerRequest {
            server_password: password(2),
        })
        .await?;
        assert_eq!(auth.state(), AuthState::SignedOut);
        register(&auth, "private-user").await?;
        let session = login(&auth, "private-user").await?;
        assert_eq!(auth.state(), AuthState::Ready);
        let generation = online(&session).await?;
        let account_auth = session.auth();
        let before = account_auth.connected_devices().await?;
        assert_eq!(before.value.connections.as_ref().map(Vec::len), Some(1));
        let mut states = session.subscribe_connection();
        account_auth.refresh().await?;
        wait_for(&mut states, |status| {
            status.state == ConnectionState::Online && status.generation.get() > generation.get()
        })
        .await?;
        assert_eq!(account_auth.state(), AuthState::Ready);
        assert!(matches!(
            session.current(before),
            Err(ClientError::Superseded)
        ));
        let after = account_auth.connected_devices().await?;
        assert_eq!(after.value.connections.as_ref().map(Vec::len), Some(1));
        let mut events = session.subscribe_events();
        account_auth.logout().await?;
        assert_eq!(account_auth.state(), AuthState::SignedOut);
        assert!(session.is_closed());
        assert!(matches!(
            events.next().await,
            Err(SessionEventError::Closed)
        ));
        assert!(matches!(
            account_auth.connected_devices().await,
            Err(ClientError::Closed)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn password_reset_is_single_use_and_replaces_the_accepted_password() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "reset-user").await?;
        let token = fixture
            .server()
            .issue_password_reset("reset-user")
            .await
            .ok_or("reset capability missing")?;
        auth.password_reset(PasswordResetConsumeRequest {
            token: token.clone(),
            password: password(1),
        })
        .await?;
        let reused = auth
            .password_reset(PasswordResetConsumeRequest {
                token,
                password: password(2),
            })
            .await;
        assert!(matches!(
            reused,
            Err(ClientError::Api(papo_api::Error::Problem {
                status: 410,
                ..
            }))
        ));
        assert!(login(&auth, "reset-user").await.is_err());
        assert_eq!(auth.state(), AuthState::SignedOut);
        let session = auth
            .login(
                LoginRequest {
                    username: "reset-user".into(),
                    password: password(1),
                },
                UnixMicros(0),
            )
            .await?;
        online(&session).await?;
        assert_eq!(session.auth().state(), AuthState::Ready);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn device_revocation_is_account_scoped_and_refresh_reports_expired_credentials() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let server = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "device-one").await?;
        register(&auth, "device-two").await?;
        let first = login(&auth, "device-one").await?;
        let second = login(&handle.auth(server)?, "device-two").await?;
        online(&first).await?;
        online(&second).await?;
        let first_auth = first.auth();
        let devices = first_auth.connected_devices().await?;
        assert_eq!(devices.value.connections.as_ref().map(Vec::len), Some(1));
        let other_devices = second.auth().connected_devices().await?;
        assert_eq!(
            other_devices.value.connections.as_ref().map(Vec::len),
            Some(1)
        );
        let dropped = first_auth
            .drop_connection(DropConnectionRequest {
                connection_id: "ALL".into(),
            })
            .await?;
        assert_eq!(dropped.value.dropped, Some(1));
        assert!(matches!(
            first_auth.refresh().await,
            Err(ClientError::Api(papo_api::Error::Problem {
                status: 401,
                ..
            }))
        ));
        assert_eq!(first_auth.state(), AuthState::Expired);
        assert!(!second.is_closed());
        assert!(second.auth().connected_devices().await.is_ok());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn failed_server_probe_is_not_committed_to_the_registry() -> Result {
    let fixture = Fixture::new(Config {
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 503,
                slug: "internal".into(),
            }),
        }],
        ..Config::default()
    })?;
    fixture.run(async {
        let handle = fixture.client().handle();
        assert!(matches!(
            handle
                .add_server(&fixture.server().base_url(), UnixMicros(0))
                .await,
            Err(ClientError::Api(papo_api::Error::Problem {
                status: 503,
                ..
            }))
        ));
        assert!(handle.subscribe_registry().borrow().servers.is_empty());
        assert!(
            handle
                .add_server(&fixture.server().base_url(), UnixMicros(0))
                .await
                .is_ok()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
