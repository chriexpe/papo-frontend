//! Linux SIGKILL crash-durability tests for catch-up intervals, `clear_catch_up` and saved positions.

#![cfg(all(target_os = "linux", feature = "test-support"))]

mod fixture;

use base64::{Engine, engine::general_purpose::STANDARD};
use fixture::{Fixture, Result, login, register};
use papo_api::HistoryQuery;
use papo_sdk::{
    AuthService, Backoff, Client, ClientConfig, DataRoot, Jitter, ReadingPosition, UnixMicros,
    Uuid, VisibilityContinuity,
    secret::{
        CredentialGeneration, SecretKey, SecretStore, SecretValue, test_store::ScriptedSecretStore,
    },
};
use papo_test_server::{Config, HistorySeed};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::process::ExitStatusExt,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";
const FIRST: &str = "70000000-0000-4000-8000-000000000001";

struct NoJitter;
impl Jitter for NoJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}

fn channel() -> Uuid {
    CHANNEL.parse().unwrap()
}

fn server_config() -> Config {
    Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: AUTHOR.into(),
            first_message_id: FIRST.into(),
            count: 10,
            first_created_at: "2026-01-01T00:00:00.123450Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        }],
        ..Config::default()
    }
}

#[test]
fn repeated_sigkills_preserve_acknowledged_catch_up_clears_and_saved_positions() -> Result {
    for mode in ["catch_up_ack", "clear_ack", "position_burst"] {
        run_kill_mode(mode)?;
    }
    Ok(())
}

fn run_kill_mode(mode: &str) -> Result {
    let mut f = Fixture::new(server_config())?;
    let (saved, secret, first_id, sixth_id, tenth_id) = f.run(async {
        let handle = f.client().handle();
        let server = handle
            .add_server(&f.server().base_url(), UnixMicros(0))
            .await?;
        let auth: AuthService = handle.auth(server)?;
        register(&auth, "killcatch").await?;
        let session = login(&auth, "killcatch").await?;
        fixture::online(&session).await?;
        let history = session.history(channel()).await?;
        history
            .page(HistoryQuery {
                limit: Some(20),
                ..HistoryQuery::default()
            })
            .await?;
        let snap = history.subscribe().await?.snapshot;
        let saved = handle.subscribe_registry().borrow().accounts[0].clone();
        let key = SecretKey::new(saved.id, saved.credential_generation);
        let secret = f.store.snapshot(key).ok_or("missing stored secret")?;
        let id_at = |i: usize| snap.messages[i].id.clone().unwrap();
        if mode == "clear_ack" {
            let ids = [id_at(0).parse()?];
            session
                .report_visible(
                    channel(),
                    snap.generation,
                    snap.version,
                    &ids,
                    VisibilityContinuity::Continuous,
                )
                .await?;
            let tail = [id_at(8).parse()?, id_at(9).parse()?];
            session
                .report_visible(
                    channel(),
                    snap.generation,
                    snap.version,
                    &tail,
                    VisibilityContinuity::Discontinuous,
                )
                .await?;
        }
        Ok::<_, Box<dyn std::error::Error>>((
            saved,
            STANDARD.encode(secret.expose()),
            id_at(0),
            id_at(5),
            id_at(9),
        ))
    })??;
    f.close_client()?;

    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "catch_up_kill_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PAPO_CU_ROOT", f.profile.path())
        .env("PAPO_CU_MODE", mode)
        .env("PAPO_CU_USER", saved.id.user().to_string())
        .env("PAPO_CU_GEN", saved.credential_generation.get().to_string())
        .env("PAPO_CU_SECRET", secret)
        .env("PAPO_CU_URL", f.server().base_url())
        .env("PAPO_CU_M1", &first_id)
        .env("PAPO_CU_M6", &sixth_id)
        .env("PAPO_CU_M10", &tenth_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let reader = BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
    let mut acked_offset_units: Option<i64> = None;
    let mut saw_done = false;
    for line in reader.lines() {
        let line = line?;
        if let Some((_, off)) = line.split_once("PAPO_POS_ACK ") {
            acked_offset_units = Some(off.trim().parse()?);
        }
        if line.contains("PAPO_KILL_NOW") {
            saw_done = true;
            child.kill()?;
            break;
        }
    }
    assert!(saw_done, "child exited before PAPO_KILL_NOW in {mode}");
    assert_eq!(child.wait()?.signal(), Some(9));
    verify_after_kill(f, mode, saved.id, &first_id, acked_offset_units)
}

fn verify_after_kill(
    mut f: Fixture,
    mode: &str,
    account: papo_sdk::AccountId,
    first_id: &str,
    acked_offset_units: Option<i64>,
) -> Result {
    let db = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    assert_eq!(integrity, "ok");
    assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);

    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        match mode {
            "catch_up_ack" => {
                let gap = session
                    .catch_up_state(channel())
                    .and_then(|s| s.interval)
                    .expect("acknowledged catch-up interval must survive SIGKILL");
                assert_eq!(
                    gap.start_after.unwrap().message_id,
                    first_id.parse::<Uuid>()?
                );
            }
            "clear_ack" => {
                assert!(
                    session
                        .catch_up_state(channel())
                        .and_then(|s| s.interval)
                        .is_none(),
                    "cleared catch-up interval must remain deleted after SIGKILL"
                );
            }
            "position_burst" => {
                let pos = session
                    .reading_position(channel())
                    .expect("acknowledged reading position must survive SIGKILL");
                assert!(acked_offset_units.is_some());
                assert!(pos.offset_units >= -50);
            }
            _ => unreachable!(),
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
#[ignore = "child process for Session 13b SIGKILL durability test"]
fn catch_up_kill_child() {
    let root = std::env::var_os("PAPO_CU_ROOT").unwrap();
    let mode = std::env::var("PAPO_CU_MODE").unwrap();
    let user: Uuid = std::env::var("PAPO_CU_USER").unwrap().parse().unwrap();
    let cred_gen: u64 = std::env::var("PAPO_CU_GEN").unwrap().parse().unwrap();
    let secret = STANDARD
        .decode(std::env::var("PAPO_CU_SECRET").unwrap())
        .unwrap();
    let url = std::env::var("PAPO_CU_URL").unwrap();
    let m1: Uuid = std::env::var("PAPO_CU_M1").unwrap().parse().unwrap();
    let m6: Uuid = std::env::var("PAPO_CU_M6").unwrap().parse().unwrap();
    let m10: Uuid = std::env::var("PAPO_CU_M10").unwrap().parse().unwrap();

    let store = Arc::new(ScriptedSecretStore::default());
    let server_id = papo_sdk::ServerAddress::parse(&url).unwrap().id();
    let account = papo_sdk::AccountId::new(server_id, user);
    let key = SecretKey::new(account, CredentialGeneration::new(cred_gen));

    let cfg = ClientConfig {
        insecure_loopback: true,
        jitter: Arc::new(NoJitter),
        backoff: Backoff {
            initial: Duration::from_secs(2),
            maximum: Duration::from_secs(2),
        },
        ..ClientConfig::default()
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(store.write(key, SecretValue::new(secret)))
        .unwrap();
    drop(rt);
    let client = Client::open(DataRoot::new(root), store, cfg).unwrap();
    client
        .block_on(async {
            let session = client.handle().session(account).unwrap();
            fixture::online(&session).await.unwrap();
            let history = session.history(channel()).await.unwrap();
            history
                .page(HistoryQuery {
                    limit: Some(20),
                    ..HistoryQuery::default()
                })
                .await
                .unwrap();
            let snap = history.subscribe().await.unwrap().snapshot;
            match mode.as_str() {
                "catch_up_ack" => {
                    session
                        .report_visible(
                            channel(),
                            snap.generation,
                            snap.version,
                            &[m1],
                            VisibilityContinuity::Continuous,
                        )
                        .await
                        .unwrap();
                    session
                        .report_visible(
                            channel(),
                            snap.generation,
                            snap.version,
                            &[m10],
                            VisibilityContinuity::Discontinuous,
                        )
                        .await
                        .unwrap();
                }
                "clear_ack" => {
                    session.clear_catch_up(channel()).await.unwrap();
                }
                "position_burst" => {
                    let mut set = tokio::task::JoinSet::new();
                    for step in 0..8_i64 {
                        let s = session.clone();
                        set.spawn(async move {
                            let off = step * 11 - 50;
                            s.save_reading_position(
                                channel(),
                                Some(ReadingPosition {
                                    message_id: m6,
                                    offset_units: off,
                                }),
                            )
                            .await
                            .unwrap();
                            println!("PAPO_POS_ACK {off}");
                            let _ = std::io::stdout().flush();
                        });
                    }
                    let _ = set.join_next().await;
                }
                _ => unreachable!(),
            }
            println!("PAPO_KILL_NOW");
            let _ = std::io::stdout().flush();
        })
        .unwrap();
    std::process::abort();
}
