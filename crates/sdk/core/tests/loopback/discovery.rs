//! Proves server discovery through `add_server` and the non-saving `discover` over loopback;
//! wire parsing and failure categories are pinned in the SDK's unit tests.

use papo_sdk::{DiscoveryFailure, UnixMicros};
use papo_test_server::{Config, Failure, RestStep};

use super::fixture::{Fixture, Result, password};

fn failing_server(status: u16, slug: &str) -> Config {
    Config {
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status,
                slug: slug.into(),
            }),
        }],
        ..Config::default()
    }
}

#[test]
fn adding_a_server_returns_what_it_says_about_itself() -> Result {
    let fixture = Fixture::new(Config {
        server_name: "Lumen".into(),
        ..Config::default()
    })?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let added = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        assert_eq!(added.discovery.name.as_deref(), Some("Lumen"));
        assert!(added.discovery.public);
        assert_eq!(added.discovery.icon, None);
        let registry = handle.subscribe_registry();
        let saved = registry.borrow();
        assert_eq!(saved.servers.len(), 1);
        assert_eq!(saved.servers[0].address.id(), added.id);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_private_server_says_so_before_any_sign_in() -> Result {
    let fixture = Fixture::new(Config {
        public: false,
        server_password: Some(password(1)),
        ..Config::default()
    })?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let discovery = handle.discover(&fixture.server().base_url()).await?;
        assert!(!discovery.public);
        let added = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        assert!(!added.discovery.public);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn discovering_a_server_saves_nothing() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let before = handle.subscribe_registry().borrow().version;
        let discovery = handle.discover(&fixture.server().base_url()).await?;
        assert_eq!(discovery.name.as_deref(), Some("Papo test backend"));
        let registry = handle.subscribe_registry();
        assert_eq!(registry.borrow().version, before, "no registry commit");
        assert_eq!(registry.borrow().servers.len(), 0);
        // Already saved: discovery still answers, for "You already added …".
        handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await?;
        let again = handle.discover(&fixture.server().base_url()).await?;
        assert_eq!(again, discovery);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_server_not_set_up_is_told_apart_and_not_saved() -> Result {
    let fixture = Fixture::new(Config {
        server_exists: false,
        ..Config::default()
    })?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let error = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await
            .unwrap_err();
        assert_eq!(
            DiscoveryFailure::classify(&error),
            Some(DiscoveryFailure::NotSetUp)
        );
        assert_eq!(handle.subscribe_registry().borrow().servers.len(), 0);
        let error = handle
            .discover(&fixture.server().base_url())
            .await
            .unwrap_err();
        assert_eq!(
            DiscoveryFailure::classify(&error),
            Some(DiscoveryFailure::NotSetUp)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_failing_server_is_unreachable_and_not_saved() -> Result {
    let fixture = Fixture::new(failing_server(503, "internal"))?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let error = handle
            .add_server(&fixture.server().base_url(), UnixMicros(0))
            .await
            .unwrap_err();
        assert_eq!(
            DiscoveryFailure::classify(&error),
            Some(DiscoveryFailure::Unreachable)
        );
        assert_eq!(handle.subscribe_registry().borrow().servers.len(), 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_404_without_papos_not_found_problem_is_not_papo() -> Result {
    let fixture = Fixture::new(failing_server(404, "nothing-here"))?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let error = handle
            .discover(&fixture.server().base_url())
            .await
            .unwrap_err();
        assert_eq!(
            DiscoveryFailure::classify(&error),
            Some(DiscoveryFailure::NotPapo)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn nobody_listening_is_unreachable() -> Result {
    let fixture = Fixture::new(Config::default())?;
    // A port that was just free: nothing accepts connections on it.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    fixture.run(async {
        let handle = fixture.client().handle();
        let error = handle
            .discover(&format!("http://{closed}"))
            .await
            .unwrap_err();
        assert_eq!(
            DiscoveryFailure::classify(&error),
            Some(DiscoveryFailure::Unreachable)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
