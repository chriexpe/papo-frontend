//! Runs session 5b live lists against a live endpoint or fixture; the SDK owns the runtime.
//!
//! Run `cargo run -p papo-sdk-ui --example sdk_session -- --fixture`.
//! Live mode reads `PAPO_SERVER`, `PAPO_USER`, `PAPO_PASSWORD` and optional `PAPO_SERVER_PASSWORD`,
//! then waits for an externally induced network drop and reconnect. The in-memory credential
//! capability is demonstration-only, not an OS credential store; the temporary profile is removed.

use std::{env, error::Error, process::ExitCode, sync::Arc, time::Duration};

use papo_sdk::{
    Client, ClientConfig, ConnectionState, ConnectionStatus, DataRoot, Generation, LoginRequest,
    LoginServerRequest, RegisterRequest, Session, UnixMicros,
    secret::test_store::ScriptedSecretStore,
};
use papo_sdk_ui::{ChannelListItem, ChannelListService, ChannelListSubscription};
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
            insecure_loopback: fixture || env::var_os("PAPO_LOOPBACK_HTTP").is_some(),
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
    let mut channels = ChannelListService::new(session.clone()).subscribe();
    println!("Channels {}", channels.snapshot.len());
    if let Some(server) = fixture {
        exercise_channels(server, &session, &mut status, &mut channels).await?;
    } else {
        println!("Waiting for external channel changes and network drop");
        loop {
            tokio::select! {
                update = std::future::poll_fn(|cx| channels.stream.as_mut().poll_next(cx)) => {
                    let update = update.ok_or("channel stream closed")?;
                    for diff in update.diffs { diff.apply(&mut channels.snapshot); }
                    channels.version = update.version;
                    channels.generation = update.generation;
                    println!("Channels {} version {} generation {}", channels.snapshot.len(), channels.version, channels.generation.get());
                }
                changed = status.changed() => {
                    changed?;
                    if matches!(status.borrow_and_update().state, ConnectionState::Backoff { .. }) { break; }
                }
            }
        }
        let online = wait_for(&mut status, |state| state == ConnectionState::Online).await?;
        require_new_generation(&session, first.generation, online.generation)?;
        wait_channels(&mut channels, |list| list.generation == online.generation).await?;
        println!("Reconciled {}", online.generation.get());
    }
    Ok(())
}

async fn exercise_channels(
    server: &TestServer,
    session: &Session,
    status: &mut watch::Receiver<ConnectionStatus>,
    channels: &mut ChannelListSubscription,
) -> Result<()> {
    let first = session.generation();
    let created = server.create_channel("fixture-created").await?;
    let id = created
        .id
        .as_deref()
        .ok_or("missing fixture channel identity")?;
    wait_channels(channels, |list| contains(list, id, Some("fixture-created"))).await?;
    if !server.rename_channel(id, "fixture-renamed").await {
        return Err("fixture rename failed".into());
    }
    wait_channels(channels, |list| contains(list, id, Some("fixture-renamed"))).await?;
    if !server.delete_channel(id).await {
        return Err("fixture delete failed".into());
    }
    wait_channels(channels, |list| !contains(list, id, None)).await?;
    println!("Live create rename delete verified");

    let renamed = server.create_channel("fixture-before-drop").await?;
    let removed = server.create_channel("fixture-remove-offline").await?;
    let renamed_id = renamed
        .id
        .as_deref()
        .ok_or("missing fixture channel identity")?;
    let removed_id = removed
        .id
        .as_deref()
        .ok_or("missing fixture channel identity")?;
    wait_channels(channels, |list| {
        contains(list, renamed_id, None) && contains(list, removed_id, None)
    })
    .await?;
    server.set_gateway_accepting(false).await;
    server.disconnect_gateways();
    let backoff = wait_for(status, |state| {
        matches!(state, ConnectionState::Backoff { .. })
    })
    .await?;
    println!("Backoff {}", backoff.generation.get());
    let offline = server.create_channel("fixture-offline-created").await?;
    let offline_id = offline
        .id
        .as_deref()
        .ok_or("missing fixture channel identity")?;
    if !server
        .rename_channel(renamed_id, "fixture-offline-renamed")
        .await
        || !server.delete_channel(removed_id).await
    {
        return Err("fixture offline mutation failed".into());
    }
    server.set_gateway_accepting(true).await;
    let online = wait_for(status, |state| state == ConnectionState::Online).await?;
    require_new_generation(session, first, online.generation)?;
    wait_channels(channels, |list| {
        list.generation == online.generation
            && contains(list, offline_id, Some("fixture-offline-created"))
            && contains(list, renamed_id, Some("fixture-offline-renamed"))
            && !contains(list, removed_id, None)
    })
    .await?;
    println!(
        "Reconciled create rename delete generation {}",
        online.generation.get()
    );
    Ok(())
}

fn contains(list: &ChannelListSubscription, id: &str, name: Option<&str>) -> bool {
    list.snapshot.iter().any(|item: &Arc<ChannelListItem>| {
        item.id.as_deref() == Some(id) && name.is_none_or(|name| item.name.as_deref() == Some(name))
    })
}

async fn wait_channels(
    channels: &mut ChannelListSubscription,
    predicate: impl Fn(&ChannelListSubscription) -> bool,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(300), async {
        while !predicate(channels) {
            let update = std::future::poll_fn(|cx| channels.stream.as_mut().poll_next(cx))
                .await
                .ok_or("channel stream closed")?;
            if update.version <= channels.version {
                return Err("channel version did not advance".into());
            }
            for diff in update.diffs {
                diff.apply(&mut channels.snapshot);
            }
            channels.version = update.version;
            channels.generation = update.generation;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await?
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
