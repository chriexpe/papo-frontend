//! Linux SIGKILL acceptance gates through real SDK enqueue/dispatch/restart; never a timer approximation.

#![cfg(all(target_os = "linux", feature = "test-support"))]
#[path = "outgoing_kill/child.rs"]
mod child;
mod fixture;
use base64::{Engine, engine::general_purpose::STANDARD};
use fixture::{Fixture, Result, login, register};
use papo_sdk::{OperationState, UnixMicros, Uuid, secret::SecretKey};
use papo_test_server::Config;
use std::{process::Command, time::Duration};
#[path = "outgoing_kill/process.rs"]
mod process;
use process::Process;

fn local(n: u128) -> Uuid {
    Uuid::from_u128(0xc000_0000_0000_4000_8000_0000_0000_0000 + n)
}

#[test]
fn repeated_sigkills_cover_queued_claimed_inflight_accepted_and_committed_sends() -> Result {
    // Three real kills at each controlled boundary exercise restart, not graceful Drop.
    for _ in 0..3 {
        for mode in [
            "queued", "claimed", "inflight", "accepted", "response", "sent",
        ] {
            interrupt(mode)?;
        }
    }
    Ok(())
}

fn interrupt(mode: &str) -> Result {
    let (mut f, saved, channels, secret) = prepare()?;
    let mut hold = f.run(async {
        match mode {
            "inflight" => Ok(Some(
                f.server().hold_next_request("POST", "/messages").await?,
            )),
            "accepted" => Ok(Some(f.server().hold_next_rest("POST", "/messages").await?)),
            _ => Ok::<_, papo_test_server::Error>(None),
        }
    })??;
    f.close_client()?;
    let control = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--ignored",
            "--exact",
            "child::outgoing_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PAPO_KILL_ROOT", f.profile.path())
        .env("PAPO_KILL_MODE", mode)
        .env("PAPO_KILL_ACCOUNT", saved.id.user().to_string())
        .env(
            "PAPO_KILL_GENERATION",
            saved.credential_generation.get().to_string(),
        )
        .env("PAPO_KILL_SECRET", secret)
        .env("PAPO_KILL_CHANNEL_A", &channels[0])
        .env("PAPO_KILL_CHANNEL_B", &channels[1]);
    let mut process = Process::start(command)?;
    process.ready(!matches!(mode, "inflight" | "accepted"))?;
    if let Some(hold) = hold.as_mut() {
        control.block_on(async {
            tokio::time::timeout(Duration::from_secs(30), hold.wait_started()).await
        })??;
    }
    process.kill()?;
    if let Some(hold) = hold {
        hold.release();
    }
    check_integrity(&f, mode)?;
    f.reopen()?;
    f.run(verify_restart(&f, saved.id, &channels[0], mode))??;
    f.finish()
}

fn prepare() -> Result<(Fixture, papo_sdk::SavedAccount, [String; 2], String)> {
    let f = Fixture::new(Config::default())?;
    let (saved, channels, secret) = f.run(async {
        let handle = f.client().handle();
        let server = handle
            .add_server(&f.server().base_url(), UnixMicros(0))
            .await?;
        let auth = handle.auth(server)?;
        register(&auth, "killproof").await?;
        let a = f.server().create_channel("a").await?.id.unwrap();
        let b = f.server().create_channel("b").await?.id.unwrap();
        let session = login(&auth, "killproof").await?;
        fixture::online(&session).await?;
        let saved = handle.subscribe_registry().borrow().accounts[0].clone();
        let key = SecretKey::new(saved.id, saved.credential_generation);
        let secret = f
            .store
            .snapshot(key)
            .ok_or("missing ephemeral credential")?;
        Ok::<_, Box<dyn std::error::Error>>((saved, [a, b], STANDARD.encode(secret.expose())))
    })??;
    Ok((f, saved, channels, secret))
}

fn check_integrity(f: &Fixture, mode: &str) -> Result {
    let db = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    assert_eq!(integrity, "ok");
    let state: String = db.query_row(
        "SELECT state FROM operations WHERE local_id=?1",
        [local(1).as_bytes()],
        |r| r.get(0),
    )?;
    assert_eq!(
        state,
        if mode == "queued" {
            "Queued"
        } else if mode == "sent" {
            "Sent"
        } else {
            "Sending"
        }
    );
    assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |row| row
            .get::<_, u32>(0))?,
        3
    );
    for table in ["operations", "drafts"] {
        let column = if table == "operations" {
            "payload"
        } else {
            "body"
        };
        let mut rows = db.prepare(&format!(
            "SELECT {column} FROM {table} WHERE {column} IS NOT NULL"
        ))?;
        for bytes in rows.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            assert!(!bytes?.windows(4).any(|part| part == b"AP8B"));
        }
    }
    drop(db);
    Ok(())
}

async fn verify_restart(
    f: &Fixture,
    account: papo_sdk::AccountId,
    channel: &str,
    mode: &str,
) -> Result {
    let session = f.client().handle().session(account)?;
    fixture::online(&session).await?;
    let queue = session.outgoing();
    let mut image = queue.subscribe().await?;
    loop {
        let done = image
            .borrow_and_update()
            .operations
            .iter()
            .any(|op| op.id == local(3) && op.state == OperationState::Sent);
        if done {
            break;
        }
        image.changed().await?;
    }
    let values = image.borrow().clone();
    assert_eq!(values.operations.len(), 3);
    let first = values
        .operations
        .iter()
        .find(|op| op.id == local(1))
        .unwrap();
    let second = values
        .operations
        .iter()
        .find(|op| op.id == local(2))
        .unwrap();
    let papo_sdk::OperationKind::Message(message) = first.kind.as_ref() else {
        panic!("message expected")
    };
    assert_eq!(message.attachments.len(), 1);
    if matches!(mode, "queued" | "sent") {
        assert_eq!(message.attachments[0].bytes, [] as [u8; 0]);
        assert_eq!(
            first
                .message
                .as_ref()
                .unwrap()
                .attachments
                .as_ref()
                .unwrap()[0]
                .size_bytes,
            Some(3)
        );
        assert_eq!(first.state, OperationState::Sent);
        assert_eq!(second.state, OperationState::Sent);
        let first_at = chrono::DateTime::parse_from_rfc3339(
            first
                .message
                .as_ref()
                .unwrap()
                .created_at
                .as_deref()
                .unwrap(),
        )?;
        let second_at = chrono::DateTime::parse_from_rfc3339(
            second
                .message
                .as_ref()
                .unwrap()
                .created_at
                .as_deref()
                .unwrap(),
        )?;
        assert!(first_at < second_at);
        assert_eq!(f.server().outgoing_request_counts().await.accepted, 3);
    } else {
        assert_eq!(message.attachments[0].bytes, [0, 255, 1]);
        assert_eq!(first.state, OperationState::Unknown);
        assert_eq!(first.attempts, 1);
        assert_eq!(second.state, OperationState::Queued);
        let counts = f.server().outgoing_request_counts().await;
        assert_eq!(counts.requests, if mode == "claimed" { 1 } else { 2 });
        // During dispatch the buffered body may or may not survive process death.
        if matches!(mode, "accepted" | "response") {
            assert_eq!(counts.accepted, 2);
        } else {
            assert!(counts.accepted <= 2);
        }
        assert_eq!(queue.draft(channel.parse()?).await?.unwrap().revision, 1);
    }
    Ok(())
}
