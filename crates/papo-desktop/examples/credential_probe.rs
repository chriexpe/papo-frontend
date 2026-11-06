//! Opt-in native-store probe using only process-scoped synthetic credentials.
//!
//! Run `cargo run --locked -p papo-desktop --example credential_probe` in a
//! desktop session. Fresh adapters verify persistence rather than memory hits.
//! No server, account, application data folder or plaintext file is opened.
//! Diagnostic logs are for this probe only, not production user notification.

use gpui_kit::{AppContext as _, AsyncApp, QuitMode};
use papo_core::storage::{Secret, SecretStore, StorageError, StorageResult};
use papo_desktop::credentials::CredentialService;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy)]
enum Phase {
    Reset,
    Seed,
    Replace,
    Delete,
    CheckRemoved,
    CheckEmpty,
}

fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,credential_probe=info"),
    )
    .init();
    let passed = Arc::new(AtomicBool::new(false));
    let result = Arc::clone(&passed);
    gpui_kit::application().run(move |cx| {
        cx.set_quit_mode(QuitMode::Explicit);
        log::info!("credential_probe: display backend {}, GPU not used (no window)", cx.compositor_name());
        cx.spawn(async move |cx| {
            let first = format!("synthetic-credential-probe-{}-a", std::process::id());
            let second = format!("synthetic-credential-probe-{}-b", std::process::id());
            let mut success = true;
            for phase in [Phase::Reset, Phase::Seed, Phase::Replace, Phase::Delete, Phase::CheckRemoved] {
                if let Err(error) = run_phase(cx, &first, &second, phase).await {
                    log::error!("credential_probe: verification failed: {error}");
                    success = false;
                    break;
                }
            }
            // Both secret kinds on both probe keys are attempted even after failure.
            for phase in [Phase::Reset, Phase::CheckEmpty] {
                if let Err(error) = run_phase(cx, &first, &second, phase).await {
                    log::error!("credential_probe: cleanup failed: {error}");
                    success = false;
                }
            }
            result.store(success, Ordering::Release);
            if success { log::info!("credential_probe: PASS; persisted CRUD, isolation, overwrite and cleanup verified"); }
            cx.update(|cx| cx.quit());
        }).detach();
    });
    if passed.load(Ordering::Acquire) {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

async fn run_phase(
    cx: &mut AsyncApp,
    first: &str,
    second: &str,
    phase: Phase,
) -> StorageResult<()> {
    let service = cx.update(|cx| {
        CredentialService::new(cx, |failure, _| {
            log::warn!(
                "credential_probe: {:?} failed: {}",
                failure.operation(),
                failure.error()
            );
        })
    });
    let store = service.store();
    let first = first.to_owned();
    let second = second.to_owned();
    let (reply, received) = async_channel::bounded(1);
    let worker = std::thread::Builder::new()
        .name("credential-probe".into())
        .spawn(move || {
            let _ = reply.try_send(exercise(store.as_ref(), &first, &second, phase));
        })
        .map_err(|_| StorageError::new("probe worker could not start"))?;
    let outcome = received
        .recv()
        .await
        .map_err(|_| StorageError::new("probe worker did not reply"));
    let joined = cx.background_spawn(async move { worker.join() }).await;
    if joined.is_err() {
        return Err(StorageError::new("probe worker panicked"));
    }
    outcome?
}

fn exercise(store: &dyn SecretStore, first: &str, second: &str, phase: Phase) -> StorageResult<()> {
    match phase {
        Phase::Reset => {
            let first = store.forget_server(first);
            let second = store.forget_server(second);
            first.and(second)
        }
        Phase::Seed => {
            store.store(first, Secret::SessionToken, "synthetic-old")?;
            store.store(first, Secret::ServerPassword, "synthetic-password")?;
            store.store(second, Secret::SessionToken, "synthetic-other")
        }
        Phase::Replace => {
            check(
                store.load(first, Secret::SessionToken)?.as_deref(),
                Some("synthetic-old"),
            )?;
            check(
                store.load(first, Secret::ServerPassword)?.as_deref(),
                Some("synthetic-password"),
            )?;
            check(
                store.load(second, Secret::SessionToken)?.as_deref(),
                Some("synthetic-other"),
            )?;
            store.store(first, Secret::SessionToken, "synthetic-new")
        }
        Phase::Delete => {
            check(
                store.load(first, Secret::SessionToken)?.as_deref(),
                Some("synthetic-new"),
            )?;
            store.forget_server(first)
        }
        Phase::CheckRemoved => {
            check(store.load(first, Secret::SessionToken)?.as_deref(), None)?;
            check(store.load(first, Secret::ServerPassword)?.as_deref(), None)?;
            check(
                store.load(second, Secret::SessionToken)?.as_deref(),
                Some("synthetic-other"),
            )
        }
        Phase::CheckEmpty => {
            for server in [first, second] {
                for secret in [Secret::SessionToken, Secret::ServerPassword] {
                    check(store.load(server, secret)?.as_deref(), None)?;
                }
            }
            Ok(())
        }
    }
}

fn check(actual: Option<&str>, expected: Option<&str>) -> StorageResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StorageError::new(
            "probe value mismatch (contents withheld)",
        ))
    }
}
