//! Pages 10k messages both ways online and offline, then reconciles disconnected mutations.
//! Run `cargo run -p papo-sdk --features test-support --example history` for an automated fixture.
//! Live mode uses `--live` and `PAPO_SERVER`, `PAPO_USER`, `PAPO_PASSWORD`, `PAPO_CHANNEL`,
//! optional `PAPO_SERVER_PASSWORD` and `PAPO_LOOPBACK_HTTP`; induce an external gateway drop.
//! Only counts and generations are printed. Credentials live in demonstration-only memory;
//! the temporary account profile is removed on exit, never used as a production secret store.

use papo_api::{HistoryOrder, HistoryQuery, Uuid};
use papo_sdk::{
    Backoff, Client, ClientConfig, ConnectionState, DataRoot, HistoryHandle, HistorySource,
    LoginRequest, LoginServerRequest, RegisterRequest, Session, UnixMicros,
    secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, HistorySeed, TestServer};
use std::{env, process::ExitCode, sync::Arc, time::Duration};

mod history_demo;
use history_demo::{Result, demonstration_password, traverse, wait_for};

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const COUNT: usize = 10_000;

fn recipe(count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: "20000000-0000-4000-8000-000000000001".into(),
        first_message_id: "70000000-0000-4000-8000-000000000001".into(),
        count,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

fn main() -> ExitCode {
    if run().is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("Failed");
        ExitCode::FAILURE
    }
}

fn run() -> Result {
    let live = env::args().skip(1).any(|argument| argument == "--live");
    let profile = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: !live || env::var_os("PAPO_LOOPBACK_HTTP").is_some(),
            // A long backoff keeps the disconnected cache traversal in a single generation.
            backoff: Backoff {
                initial: Duration::from_secs(300),
                maximum: Duration::from_secs(300),
            },
            ..ClientConfig::default()
        },
    )?;
    let result = client.block_on(exercise(&client, live));
    let shutdown = client.shutdown();
    result??;
    let report = shutdown?;
    if !report.reads_drained || !report.state.finished || !report.shared.finished {
        return Err("incomplete shutdown".into());
    }
    println!("Shutdown");
    Ok(())
}

async fn exercise(client: &Client, live: bool) -> Result {
    let fixture = if live {
        None
    } else {
        Some(
            TestServer::start(Config {
                history: vec![recipe(COUNT)],
                ..Config::default()
            })
            .await?,
        )
    };
    let result = observe(client, fixture.as_ref()).await;
    let stopped = match fixture {
        Some(server) => server.shutdown().await,
        None => Ok(()),
    };
    result?;
    stopped?;
    Ok(())
}

async fn sign_in(client: &Client, fixture: Option<&TestServer>) -> Result<(Session, Uuid)> {
    let address = match fixture {
        Some(server) => server.base_url(),
        None => env::var("PAPO_SERVER")?,
    };
    let username = if fixture.is_some() {
        "history-demo".into()
    } else {
        env::var("PAPO_USER")?
    };
    let password = if fixture.is_some() {
        demonstration_password()
    } else {
        env::var("PAPO_PASSWORD")?
    };
    let channel = if fixture.is_some() {
        Uuid::parse_str(CHANNEL)?
    } else {
        Uuid::parse_str(&env::var("PAPO_CHANNEL")?)?
    };
    let handle = client.handle();
    let server = handle.add_server(&address, UnixMicros(0)).await?;
    let auth = handle.auth(server)?;
    if fixture.is_none()
        && let Ok(server_password) = env::var("PAPO_SERVER_PASSWORD")
    {
        auth.server_login(LoginServerRequest { server_password })
            .await?;
    }
    if fixture.is_some() {
        auth.register(RegisterRequest {
            username: username.clone(),
            password: password.clone(),
        })
        .await?;
    }
    let session = auth
        .login(LoginRequest { username, password }, UnixMicros(0))
        .await?;
    Ok((session, channel))
}

async fn observe(client: &Client, fixture: Option<&TestServer>) -> Result {
    let (session, channel) = sign_in(client, fixture).await?;
    let mut status = session.subscribe_connection();
    let first = wait_for(&mut status, |state| state == ConnectionState::Online).await?;
    println!("Online generation {}", first.generation.get());
    let history = session.history(channel).await?;
    // Prove both unbounded edges reach server authority before either traversal fills the cache.
    for order in [HistoryOrder::Desc, HistoryOrder::Asc] {
        let page = history
            .page(HistoryQuery {
                order,
                limit: Some(100),
                ..HistoryQuery::default()
            })
            .await?;
        if page.source != HistorySource::Server {
            return Err("fresh edge did not use server".into());
        }
        match order {
            HistoryOrder::Desc => println!(
                "Server descending messages {} generation {}",
                page.messages.len(),
                page.generation.get()
            ),
            HistoryOrder::Asc => println!(
                "Server ascending messages {} generation {}",
                page.messages.len(),
                page.generation.get()
            ),
        }
    }
    let descending = traverse(&history, HistoryOrder::Desc, false).await?;
    let ascending = traverse(&history, HistoryOrder::Asc, false).await?;
    let mut reversed = descending.ids.clone();
    reversed.reverse();
    if reversed != ascending.ids {
        return Err("bidirectional history mismatch".into());
    }
    if fixture.is_some() && ascending.ids.len() != COUNT {
        return Err("fixture history count mismatch".into());
    }
    println!(
        "Online messages {} server_pages {} cache_pages {} generation {}",
        ascending.ids.len(),
        ascending.server_pages + descending.server_pages,
        ascending.cache_pages + descending.cache_pages,
        session.generation().get()
    );
    if let Some(server) = fixture {
        server.set_gateway_accepting(false).await;
        server.disconnect_gateways();
    }
    let offline = wait_for(&mut status, |state| {
        matches!(state, ConnectionState::Backoff { .. })
    })
    .await?;
    println!("Backoff generation {}", offline.generation.get());
    let cached_desc = traverse(&history, HistoryOrder::Desc, true).await?;
    let cached_asc = traverse(&history, HistoryOrder::Asc, true).await?;
    if cached_desc.ids != descending.ids || cached_asc.ids != ascending.ids {
        return Err("offline cache history mismatch".into());
    }
    println!(
        "Offline messages {} cache_pages {} generation {}",
        cached_asc.ids.len(),
        cached_asc.cache_pages + cached_desc.cache_pages,
        session.generation().get()
    );
    if let Some(server) = fixture {
        mutate_disconnected(server).await?;
        server.set_gateway_accepting(true).await;
    }
    session.reconnect()?;
    let online = wait_for(&mut status, |state| state == ConnectionState::Online).await?;
    if online.generation.get() <= first.generation.get() {
        return Err("generation did not advance".into());
    }
    let page = history
        .page(HistoryQuery {
            limit: Some(100),
            ..HistoryQuery::default()
        })
        .await?;
    if fixture.is_some() {
        verify_reconciled(&history, &page).await?;
    }
    println!(
        "Reconciled messages {} pins {} generation {}",
        page.messages.len(),
        history.pins().await?.value.len(),
        online.generation.get()
    );
    Ok(())
}

async fn mutate_disconnected(server: &TestServer) -> Result {
    let messages = recipe(COUNT + 3).messages()?;
    let edited = messages[COUNT - 3]
        .id
        .as_deref()
        .ok_or("missing fixture identity")?;
    let deleted = messages[COUNT - 2]
        .id
        .as_deref()
        .ok_or("missing fixture identity")?;
    server
        .edit_message(edited, "", "2026-01-01T00:00:01.123456Z")
        .await?;
    if !server.delete_message(deleted).await? {
        return Err("fixture delete failed".into());
    }
    for message in &messages[COUNT..] {
        server.publish_message(message.clone()).await?;
    }
    Ok(())
}

async fn verify_reconciled(history: &HistoryHandle, page: &papo_sdk::HistoryPage) -> Result {
    let messages = recipe(COUNT + 3).messages()?;
    let edited = &messages[COUNT - 3].id;
    let deleted = &messages[COUNT - 2].id;
    if page.messages.iter().any(|message| &message.id == deleted) {
        return Err("deleted message survived reconcile".into());
    }
    if !page.messages.iter().any(|message| {
        &message.id == edited && message.edited_at.as_deref() == Some("2026-01-01T00:00:01.123456Z")
    }) {
        return Err("disconnected edit missing".into());
    }
    for expected in &messages[COUNT..] {
        if !page
            .messages
            .iter()
            .any(|message| message.id == expected.id)
        {
            return Err("disconnected creation missing".into());
        }
    }
    let traversal = traverse(history, HistoryOrder::Desc, false).await?;
    if traversal.ids.len() != COUNT + 2 {
        return Err("reconciled total mismatch".into());
    }
    Ok(())
}
