//! Linux process-crash gate using actual SIGKILLs; power-loss and network sends are not modeled.

use std::os::unix::process::ExitStatusExt;

use crate::removal::RemovalJobs;
use crate::secret::{SecretStoreBoundary, test_store::ScriptedSecretStore};
use crate::storage_tests::{Fixture, Hooks, account, address};
use crate::storage_worker::{ReadPool, StorageWorker, WorkerConfig};
use crate::{DataRoot, Registry, UnixMicros};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn interrupt(root: &DataRoot, mode: &str, round: u32, cut: usize) -> (Vec<u32>, bool) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "crash_tests::crash_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PAPO_CRASH_ROOT", root.path())
        .env("PAPO_CRASH_MODE", mode)
        .env("PAPO_CRASH_ROUND", round.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    let mut commits = 0;
    let mut acknowledged = Vec::new();
    let mut removal_acknowledged = false;
    let mut killed = false;
    for line in output.lines() {
        let line = line.unwrap();
        if line.contains("PAPO_COMMIT") {
            commits += 1;
            if commits == cut {
                // Alternate a cut before commit with a race against actual WAL/fsync work.
                if round.is_multiple_of(2) {
                    input.write_all(b"go\n").unwrap();
                    input.flush().unwrap();
                }
                child.kill().unwrap();
                killed = true;
                break;
            }
            input.write_all(b"go\n").unwrap();
            input.flush().unwrap();
        }
        if let Some((_, index)) = line.split_once("PAPO_ACK ") {
            acknowledged.push(index.parse().unwrap());
        }
        if line.contains("PAPO_REMOVAL_ACK") {
            removal_acknowledged = true;
        }
    }
    assert!(
        killed,
        "child reached end before planned commit interruption"
    );
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9));
    (acknowledged, removal_acknowledged)
}

fn integrity(root: &DataRoot) {
    let connection = rusqlite::Connection::open(root.state()).unwrap();
    let result: String = connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(result, "ok");
    assert!(
        !connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}

#[test]
fn repeated_sigkills_preserve_every_acknowledged_full_registry_commit() {
    let mut fixture = Fixture::new();
    fixture.close();
    let mut acknowledged = Vec::new();
    for round in 0..30 {
        acknowledged
            .extend(interrupt(&fixture.root(), "registry", round, 1 + (round as usize % 4)).0);
        integrity(&fixture.root());
        let connection = rusqlite::Connection::open(fixture.root().state()).unwrap();
        for index in &acknowledged {
            let id = address(*index).id();
            assert!(
                connection
                    .query_row("SELECT 1 FROM servers WHERE id=?", [id.as_bytes()], |_| Ok(
                        ()
                    ))
                    .is_ok()
            );
        }
    }
}

#[test]
fn repeated_sigkills_during_removal_resume_before_profile_access() {
    for round in 0..18 {
        let mut fixture = Fixture::new();
        let server = address(1);
        let id = account(&server, 1);
        fixture.runtime().block_on(async {
            let registry = fixture.storage().registry();
            registry
                .add_server(server.clone(), UnixMicros(1))
                .await
                .unwrap();
            registry
                .add_account(id, None, None, true, UnixMicros(1))
                .await
                .unwrap();
        });
        let cache = fixture.root().account_cache(id);
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, []).unwrap();
        fixture.close();
        let (_, acknowledged) =
            interrupt(&fixture.root(), "removal", round, 1 + (round as usize % 9));
        integrity(&fixture.root());
        fixture.reopen();
        fixture.runtime().block_on(async {
            assert!(
                fixture
                    .storage()
                    .removals()
                    .pending()
                    .await
                    .unwrap()
                    .is_empty()
            );
            if acknowledged {
                assert!(
                    fixture
                        .storage()
                        .registry()
                        .servers()
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        });
        if acknowledged {
            assert!(!cache.exists());
        }
        integrity(&fixture.root());
    }
}

/// Launched only by the parent gate, with pipes controlling SQLite commit boundaries.
#[test]
#[ignore = "child process for repeated SIGKILL gate"]
fn crash_child() {
    let root = DataRoot::new(std::env::var_os("PAPO_CRASH_ROOT").unwrap());
    let mode = std::env::var("PAPO_CRASH_MODE").unwrap();
    let round: u32 = std::env::var("PAPO_CRASH_ROUND").unwrap().parse().unwrap();
    let lease = root.acquire().unwrap();
    let thread_lease = Arc::clone(&lease);
    let (mut state, writer) = StorageWorker::state(
        root.state(),
        move |connection| {
            let _lease = &thread_lease;
            crate::state_schema::migrate(connection).unwrap();
            connection.commit_hook(Some(|| {
                println!("PAPO_COMMIT");
                std::io::stdout().flush().unwrap();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                false
            }))?;
            Ok(())
        },
        WorkerConfig::default(),
    )
    .unwrap();
    let registry = Registry {
        writer,
        readers: Arc::new(ReadPool::default()),
        path: root.state(),
        lease,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        if mode == "registry" {
            for offset in 0..8 {
                let index = round * 100 + offset;
                registry
                    .add_server(address(index), UnixMicros(i64::from(index)))
                    .await
                    .unwrap();
                println!("PAPO_ACK {index}");
                std::io::stdout().flush().unwrap();
            }
        } else {
            let jobs = RemovalJobs {
                registry: registry.clone(),
                root: root.clone(),
                secrets: Arc::new(SecretStoreBoundary::new(
                    Arc::new(ScriptedSecretStore::default()),
                    Duration::from_secs(1),
                )),
                hooks: Arc::new(Hooks::default()),
                cancel: CancellationToken::new(),
                serial: Arc::new(tokio::sync::Mutex::new(())),
            };
            jobs.begin_server(address(1).id()).await.unwrap();
            println!("PAPO_REMOVAL_ACK");
            std::io::stdout().flush().unwrap();
            assert!(jobs.resume().await.unwrap().is_empty());
        }
    });
    state
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    state.join().unwrap();
}
