//! Runs session 5a against a live endpoint or loopback fixture; never owns a second runtime.
//!
//! Run `cargo run -p papo-sdk --features test-support --example sdk_session -- --fixture`.
//! Live mode reads `PAPO_SERVER`, `PAPO_USER`, `PAPO_PASSWORD` and optional `PAPO_SERVER_PASSWORD`,
//! then waits for an externally induced network drop and reconnect. The in-memory credential
//! capability is demonstration-only, not an OS credential store; the temporary profile is removed.

use std::{env, error::Error, process::ExitCode, sync::Arc, time::Duration};

use papo_api::wire::{LoginRequest, LoginServerRequest, RegisterRequest};
use papo_sdk::{
    Client, ClientConfig, ConnectionState, ConnectionStatus, DataRoot, Generation, Session,
    UnixMicros, secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, TestServer};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Credentials {
    server: String,
    username: String,
    password: String,
    server_password: Option<String>,
}

fn main() -> ExitCode {
    if run().is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("Failed");
        ExitCode::FAILURE
    }
}

fn run() -> Result<()> {
    let fixture =
        env::args().skip(1).any(|arg| arg == "--fixture") || env::var_os("PAPO_FIXTURE").is_some();
    let profile = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: fixture,
            ..ClientConfig::default()
        },
    )?;
    let operation = client.block_on(exercise(&client, fixture));
    // Keep this exit fence outside Tokio even when the session exercise fails.
    let shutdown = client.shutdown();
    operation??;
    let report = shutdown?;
    if !report.reads_drained || !report.state.finished || !report.shared.finished {
        return Err("shutdown incomplete".into());
    }
    println!("Shutdown");
    Ok(())
}

async fn exercise(client: &Client, fixture: bool) -> Result<()> {
    let server = if fixture {
        Some(TestServer::start(Config::default()).await?)
    } else {
        None
    };
    let result = observe(client, server.as_ref()).await;
    let stopped = match server {
        Some(server) => server.shutdown().await,
        None => Ok(()),
    };
    result?;
    stopped?;
    Ok(())
}

async fn observe(client: &Client, fixture: Option<&TestServer>) -> Result<()> {
    let credentials = match fixture {
        Some(server) => Credentials {
            server: server.base_url(),
            username: "session-demo".into(),
            password: demonstration_password(),
            server_password: None,
        },
        None => Credentials {
            server: env::var("PAPO_SERVER")?,
            username: env::var("PAPO_USER")?,
            password: env::var("PAPO_PASSWORD")?,
            server_password: env::var("PAPO_SERVER_PASSWORD").ok(),
        },
    };
    let handle = client.handle();
    let server = handle
        .add_server(&credentials.server, UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    if let Some(server_password) = credentials.server_password {
        auth.server_login(LoginServerRequest { server_password })
            .await?;
    }
    if fixture.is_some() {
        auth.register(RegisterRequest {
            username: credentials.username.clone(),
            password: credentials.password.clone(),
        })
        .await?;
    }
    let session = auth
        .login(
            LoginRequest {
                username: credentials.username,
                password: credentials.password,
            },
            UnixMicros(0),
        )
        .await?;
    let mut status = session.subscribe_connection();
    let first = wait_for(&mut status, |state| state == ConnectionState::Online).await?;
    println!("Online {}", first.generation.get());
    if let Some(server) = fixture {
        server.disconnect_gateways();
    }
    let backoff = wait_for(&mut status, |state| {
        matches!(state, ConnectionState::Backoff { .. })
    })
    .await?;
    println!("Backoff {}", backoff.generation.get());
    let online = wait_for(&mut status, |state| state == ConnectionState::Online).await?;
    require_new_generation(&session, first.generation, online.generation)?;
    println!("Online {}", online.generation.get());
    Ok(())
}

async fn wait_for(
    status: &mut watch::Receiver<ConnectionStatus>,
    predicate: impl Fn(ConnectionState) -> bool,
) -> Result<ConnectionStatus> {
    // This bounds a failed demonstration, not a timing assertion about reconnect policy.
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let current = *status.borrow_and_update();
            if predicate(current.state) {
                return Ok::<_, Box<dyn Error>>(current);
            }
            status.changed().await?;
        }
    })
    .await?
}

fn require_new_generation(session: &Session, old: Generation, new: Generation) -> Result<()> {
    if new.get() <= old.get() || session.generation() != new {
        return Err("generation did not advance".into());
    }
    Ok(())
}

fn demonstration_password() -> String {
    std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect()
}
