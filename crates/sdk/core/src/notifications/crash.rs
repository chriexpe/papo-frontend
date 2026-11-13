//! Actual process interruption after FULL claim and before platform handoff; no power-loss guarantee.

use super::{
    live,
    tests::{candidate, fixture},
};
use std::os::unix::process::ExitStatusExt;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::Arc,
};

#[test]
#[ignore = "subprocess entry point"]
fn notification_claim_child() {
    let Some(path) = std::env::var_os("PAPO_NOTIFICATION_CRASH_ROOT") else {
        return;
    };
    let (client, session) = fixture(std::path::Path::new(&path));
    client
        .block_on(async {
            let resources = Arc::clone(&session.inner.resources.borrow());
            let delivery =
                live::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                    .await
                    .unwrap()
                    .unwrap();
            if std::env::var_os("PAPO_NOTIFICATION_AFTER_HANDOFF").is_some() {
                live::handoff(&session, delivery).unwrap();
            }
        })
        .unwrap();
    println!("PAPO_NOTIFICATION_CLAIM_COMMITTED");
    std::io::stdout().flush().unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}

#[test]
fn sigkill_after_claim_before_or_after_handoff_never_duplicates_on_replay() {
    for after_handoff in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "notifications::crash::notification_claim_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PAPO_NOTIFICATION_CRASH_ROOT", directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if after_handoff {
            command.env("PAPO_NOTIFICATION_AFTER_HANDOFF", "1");
        }
        let mut child = command.spawn().unwrap();
        let _input = child.stdin.take().unwrap();
        let mut observed = false;
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            if line.unwrap().contains("PAPO_NOTIFICATION_CLAIM_COMMITTED") {
                observed = true;
                break;
            }
        }
        assert!(observed);
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        let connection = rusqlite::Connection::open(directory.path().join("state.sqlite")).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        drop(connection);
        let (mut client, session) = fixture(directory.path());
        client
            .block_on(async {
                let resources = Arc::clone(&session.inner.resources.borrow());
                assert!(
                    live::deliver_at(&session, session.generation(), candidate(), &resources, 0)
                        .await
                        .unwrap()
                        .is_none()
                );
            })
            .unwrap();
        client.shutdown().unwrap();
    }
}
