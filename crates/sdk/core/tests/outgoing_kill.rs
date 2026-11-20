//! Linux SIGKILL acceptance gates through real SDK enqueue/dispatch/restart; never a timer approximation.

#![cfg(all(target_os = "linux", feature = "test-support"))]
#[path = "outgoing_kill/child.rs"]
mod child;
mod fixture;
use base64::{Engine, engine::general_purpose::STANDARD};
use fixture::{Fixture, Result, login, register};
use papo_sdk::{DataRoot, OperationState, RetryDecision, UnixMicros, Uuid, secret::SecretKey};
use papo_test_server::Config;
use sha2::{Digest, Sha256};
use std::{io::Read, process::Command, time::Duration};
#[path = "outgoing_kill/process.rs"]
mod process;
use process::Process;

const UPLOAD_MID_BYTES: u64 = 2 * 1024 * 1024;

fn local(n: u128) -> Uuid {
    Uuid::from_u128(0xc000_0000_0000_4000_8000_0000_0000_0000 + n)
}

#[test]
fn repeated_sigkills_cover_staging_queued_claimed_inflight_upload_accepted_and_committed_sends()
-> Result {
    // Three real kills at each controlled boundary exercise restart, not graceful Drop.
    for _ in 0..3 {
        for mode in [
            "staging",
            "queued",
            "claimed",
            "inflight",
            "upload_mid",
            "accepted",
            "response",
            "sent",
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
            "upload_mid" => Ok(Some(
                f.server()
                    .hold_message_upload_after_bytes(256 * 1024)
                    .await?,
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
    let needs_enqueued = mode != "staging";
    let needs_probe = !matches!(mode, "inflight" | "upload_mid" | "accepted");
    process.ready(needs_enqueued, needs_probe)?;
    if let Some(hold) = hold.as_mut() {
        control.block_on(async {
            tokio::time::timeout(Duration::from_secs(30), hold.wait_started()).await
        })??;
    }
    process.kill()?;
    if let Some(hold) = hold {
        hold.release();
    }
    check_integrity(&f, saved.id, mode)?;
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
            .await?
            .id;
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

fn check_integrity(f: &Fixture, account: papo_sdk::AccountId, mode: &str) -> Result {
    let db = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    assert_eq!(integrity, "ok");
    assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?);
    if mode == "staging" {
        assert_eq!(
            db.query_row("SELECT count(*) FROM operations", [], |row| row
                .get::<_, u32>(0))?,
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM drafts", [], |row| row
                .get::<_, u32>(0))?,
            0
        );
        let staging_dir = DataRoot::new(f.profile.path()).outgoing_files(account);
        let leftovers: Vec<_> =
            std::fs::read_dir(&staging_dir)?.collect::<std::result::Result<_, _>>()?;
        assert!(
            leftovers
                .iter()
                .any(|entry| entry.file_name().to_string_lossy().starts_with(".tmp-")),
            "expected partial temp file on disk right after SIGKILL during staging"
        );
        return Ok(());
    }
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
    let recents: i64 = db.query_row("SELECT count(*) FROM expression_recent", [], |row| {
        row.get(0)
    })?;
    assert_eq!(recents, i64::from(mode == "sent"));
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
    if mode == "staging" {
        // Wait for startup recovery/collection to initialize the outgoing observable.
        while image.borrow_and_update().version == 0 {
            image.changed().await?;
        }
        assert!(image.borrow().operations.is_empty());
        assert!(image.borrow().upload_progress.is_none());
        let staging_dir = DataRoot::new(f.profile.path()).outgoing_files(account);
        assert_eq!(
            std::fs::read_dir(&staging_dir)?.count(),
            0,
            "recovery collection must reclaim leftover staging partials"
        );
        return Ok(());
    }
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
    assert_eq!(
        session.expressions().snapshot().await?.recent_gifs.len(),
        usize::from(matches!(mode, "queued" | "sent"))
    );
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
    assert!(message.attachments[0].inline_bytes().is_none());
    if matches!(mode, "queued" | "sent") {
        verify_completed_restart(f, first, second, message).await?;
    } else if mode == "upload_mid" {
        verify_upload_mid_restart(
            f, account, channel, &queue, &mut image, &values, first, second, message,
        )
        .await?;
    } else {
        let staged = message.attachments[0].staged().unwrap();
        assert_eq!(staged.account(), account);
        assert_eq!(staged.size_bytes(), 3);
        assert_eq!(
            std::fs::read(
                DataRoot::new(f.profile.path())
                    .outgoing_files(account)
                    .join(staged.digest_hex())
            )?,
            [0, 255, 1]
        );
        assert_eq!(first.state, OperationState::Unknown);
        assert!(values.upload_progress.is_none());
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

async fn verify_completed_restart(
    f: &Fixture,
    first: &papo_sdk::Operation,
    second: &papo_sdk::Operation,
    message: &papo_sdk::OutgoingMessage,
) -> Result {
    assert!(message.attachments[0].is_released());
    assert_eq!(message.attachments[0].size_bytes(), 0);
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
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn verify_upload_mid_restart(
    f: &Fixture,
    account: papo_sdk::AccountId,
    channel: &str,
    queue: &papo_sdk::OutgoingService,
    image: &mut tokio::sync::watch::Receiver<std::sync::Arc<papo_sdk::OutgoingSnapshot>>,
    values: &papo_sdk::OutgoingSnapshot,
    first: &papo_sdk::Operation,
    second: &papo_sdk::Operation,
    message: &papo_sdk::OutgoingMessage,
) -> Result {
    let staged = message.attachments[0].staged().unwrap();
    assert_eq!(staged.account(), account);
    assert_eq!(staged.size_bytes(), UPLOAD_MID_BYTES);
    assert_eq!(first.state, OperationState::Unknown);
    assert_eq!(first.attempts, 1);
    assert!(values.upload_progress.is_none());
    assert_eq!(second.state, OperationState::Queued);
    assert_eq!(queue.draft(channel.parse()?).await?.unwrap().revision, 1);
    let counts = f.server().outgoing_request_counts().await;
    // Interrupted upload_mid + unrelated channel local(3); never auto-resent.
    assert_eq!(counts.requests, 2);
    assert_eq!(counts.accepted, 1);

    // Explicit duplicate-risk retry uploads the exact immutable 2 MiB staged file.
    queue
        .retry(local(1), RetryDecision::AcceptPossibleDuplicate)
        .await?;
    loop {
        let done = image
            .borrow_and_update()
            .operations
            .iter()
            .any(|op| op.id == local(2) && op.state == OperationState::Sent);
        if done {
            break;
        }
        image.changed().await?;
    }
    let expected_sha = pattern_sha256(UPLOAD_MID_BYTES, 0x6b);
    assert_eq!(staged.digest(), &expected_sha);
    assert_eq!(f.server().outgoing_request_counts().await.accepted, 3);
    Ok(())
}

fn pattern_sha256(len: u64, seed: u8) -> [u8; 32] {
    let mut reader = child::PatternReader::new(len, seed);
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().into()
}
