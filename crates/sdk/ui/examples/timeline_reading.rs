//! Follow two windows, persist visible reading, restart, and upload without printing content.
//!
//! `cargo run -p papo-sdk-ui --example timeline_reading -- --fixture` is self-contained.
//! Live mode uses `PAPO_SERVER`/`PAPO_USER`/`PAPO_PASSWORD`, optional `PAPO_SERVER_PASSWORD`/`PAPO_CHANNEL`,
//! and `PAPO_LOOPBACK_HTTP` for a disposable loopback backend. The demonstration-only credential
//! store is retained across restart; real clients implement the platform `SecretStore` instead.

#[path = "timeline_reading/observe.rs"]
mod observe;

use papo_sdk::{
    AccountId, Client, ClientConfig, DataRoot, ReadPoint, Uuid,
    secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, HistorySeed, TestServer};
use std::{env, error::Error, process::ExitCode, sync::Arc};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Checkpoint {
    account: AccountId,
    channel: Uuid,
    point: ReadPoint,
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
    let fixture = env::args().skip(1).any(|arg| arg == "--fixture");
    // This runtime belongs only to the external fixture, not to any SDK session. Keeping the
    // fake server alive across Client shutdown is essential to exercising an actual restart.
    let fixture_runtime = if fixture {
        Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?,
        )
    } else {
        None
    };
    let server = fixture_runtime
        .as_ref()
        .map(|runtime| {
            runtime.block_on(TestServer::start(Config {
                history: vec![HistorySeed {
                    channel_id: Uuid::from_u128(100).to_string(),
                    author_id: Uuid::from_u128(101).to_string(),
                    first_message_id: Uuid::from_u128(1000).to_string(),
                    count: 100,
                    first_created_at: "2026-01-01T00:00:00.123456Z".into(),
                    timestamp_step_us: 1,
                    equal_timestamp_count: 3,
                }],
                ..Config::default()
            }))
        })
        .transpose()?;
    let profile = tempfile::tempdir()?;
    let secrets = Arc::new(ScriptedSecretStore::default());
    let config = ClientConfig {
        insecure_loopback: fixture || env::var_os("PAPO_LOOPBACK_HTTP").is_some(),
        ..ClientConfig::default()
    };
    let result = (|| {
        let checkpoint = {
            let mut client = Client::open(
                DataRoot::new(profile.path()),
                secrets.clone(),
                config.clone(),
            )?;
            let operation = client.block_on(observe::first(&client, server.as_ref()));
            let shutdown = client.shutdown();
            let checkpoint = operation??;
            require_shutdown(&shutdown?)?;
            checkpoint
        };
        if let Some(server) = &server {
            fixture_runtime
                .as_ref()
                .ok_or("fixture runtime missing")?
                .block_on(server.set_gateway_accepting(true));
        }
        {
            let mut client = Client::open(DataRoot::new(profile.path()), secrets, config)?;
            let operation = client.block_on(observe::restarted(&client, checkpoint));
            let shutdown = client.shutdown();
            operation??;
            require_shutdown(&shutdown?)?;
        }
        Ok::<_, Box<dyn Error>>(())
    })();
    let stopped = match (fixture_runtime.as_ref(), server) {
        (Some(runtime), Some(server)) => runtime.block_on(server.shutdown()),
        _ => Ok(()),
    };
    result?;
    stopped?;
    println!("Shutdown count 2");
    Ok(())
}

fn require_shutdown(report: &papo_sdk::StorageShutdown) -> Result<()> {
    if !report.reads_drained || !report.state.finished || !report.shared.finished {
        return Err("incomplete shutdown".into());
    }
    Ok(())
}
