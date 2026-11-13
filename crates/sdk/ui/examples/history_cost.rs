//! Headless public SDK/UI fixtures with deterministic work budgets and advisory durations.
//! Run `cargo run -p papo-sdk-ui --example history_cost -- history 10000`;
//! scenarios are history/disk (10000/100000 rows), residency (500 channels), reconnect (100 channels).
//! JSON Lines omit content, IDs, credentials and addresses. Byte estimates are not process RSS.

#[path = "history_cost/disk.rs"]
mod disk;
#[path = "history_cost/fixture.rs"]
mod fixture;
#[path = "history_cost/history.rs"]
mod history;
#[path = "history_cost/reconnect.rs"]
mod reconnect;
#[path = "history_cost/residency.rs"]
mod residency;

use fixture::Result;
use papo_sdk::{Client, ClientConfig, DataRoot, secret::test_store::ScriptedSecretStore};
use std::{process::ExitCode, sync::Arc, time::Duration};

fn main() -> ExitCode {
    if run().is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("History performance fixture failed");
        ExitCode::FAILURE
    }
}
fn run() -> Result {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "history".into());
    let count: usize = args.next().map_or(Ok(10_000), |value| value.parse())?;
    let profile = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            disk: if scenario == "disk" {
                papo_sdk::DiskBudget {
                    bytes: 2 * 1024 * 1024,
                    maintenance_interval: Duration::from_hours(1),
                }
            } else {
                papo_sdk::DiskBudget::default()
            },
            ..ClientConfig::default()
        },
    )?;
    let result = client.block_on(async {
        tokio::time::timeout(Duration::from_secs(240), async {
            match scenario.as_str() {
                "history" => history::run(&client, count).await,
                "residency" => residency::run(&client, 500).await,
                "reconnect" => reconnect::run(&client, 100).await,
                "disk" => disk::run(&client, count).await,
                _ => Err("unknown fixture scenario".into()),
            }
        })
        .await?
    });
    let stopped = client.shutdown();
    result??;
    let report = stopped?;
    if !report.reads_drained || !report.state.finished || !report.shared.finished {
        return Err("incomplete fixture shutdown".into());
    }
    Ok(())
}
