//! Synthetic credential regressions; no native store or real credential is used.

use super::{
    CredentialError,
    bridge::{Bridge, Operation},
    store::OsSecretStore,
};
use papo_core::storage::{Secret, SecretStore as _};
use std::sync::Arc;

fn fixture() -> (
    Arc<OsSecretStore>,
    async_channel::Receiver<super::bridge::Request>,
) {
    let (bridge, requests) = Bridge::new();
    (Arc::new(OsSecretStore::new(bridge)), requests)
}

#[test]
fn failed_write_keeps_new_value_in_memory_instead_of_reloading_old_native_value() {
    let (store, requests) = fixture();
    let worker = std::thread::spawn(move || {
        assert!(
            store
                .store("synthetic-a", Secret::SessionToken, "synthetic-new")
                .is_err()
        );
        assert_eq!(
            store
                .load("synthetic-a", Secret::SessionToken)
                .unwrap()
                .as_deref(),
            Some("synthetic-new")
        );
    });
    let request = requests.recv_blocking().unwrap();
    assert!(matches!(request.operation, Operation::Write(_)));
    request.complete(Err(CredentialError::Unavailable));
    worker.join().unwrap();
    assert!(requests.try_recv().is_err());
}

#[test]
fn failed_delete_tombstones_the_value_for_this_run() {
    let (store, requests) = fixture();
    let worker = std::thread::spawn(move || {
        assert!(store.remove("synthetic-a", Secret::SessionToken).is_err());
        assert_eq!(
            store.load("synthetic-a", Secret::SessionToken).unwrap(),
            None
        );
    });
    let request = requests.recv_blocking().unwrap();
    assert!(matches!(request.operation, Operation::Delete));
    request.complete(Err(CredentialError::Unavailable));
    worker.join().unwrap();
    assert!(requests.try_recv().is_err());
}

#[test]
fn forget_attempts_both_secret_kinds_even_when_first_delete_fails() {
    let (store, requests) = fixture();
    let worker = std::thread::spawn(move || {
        assert!(store.forget_server("synthetic-a").is_err());
        for secret in [Secret::SessionToken, Secret::ServerPassword] {
            assert_eq!(store.load("synthetic-a", secret).unwrap(), None);
        }
    });
    let first = requests.recv_blocking().unwrap();
    let first_key = first.key.clone();
    first.complete(Err(CredentialError::Unavailable));
    let second = requests.recv_blocking().unwrap();
    assert_ne!(first_key, second.key);
    second.complete(Ok(None));
    worker.join().unwrap();
}

#[test]
fn native_reads_are_isolated_by_server_and_secret_kind() {
    let (store, requests) = fixture();
    let worker = std::thread::spawn(move || {
        for (server, secret, value) in [
            ("synthetic-a", Secret::SessionToken, "synthetic-1"),
            ("synthetic-a", Secret::ServerPassword, "synthetic-2"),
            ("synthetic-b", Secret::SessionToken, "synthetic-3"),
        ] {
            assert_eq!(store.load(server, secret).unwrap().as_deref(), Some(value));
            assert_eq!(store.load(server, secret).unwrap().as_deref(), Some(value));
        }
    });
    let mut keys = std::collections::HashSet::new();
    for value in ["synthetic-1", "synthetic-2", "synthetic-3"] {
        let request = requests.recv_blocking().unwrap();
        assert!(matches!(request.operation, Operation::Read));
        assert!(keys.insert(request.key.clone()));
        assert!(
            request
                .key
                .starts_with("io.github.chriexpe.Papo.Desktop/credentials/v1/")
        );
        request.complete(Ok(Some(value.to_owned())));
    }
    worker.join().unwrap();
    assert!(requests.try_recv().is_err());
}

#[test]
fn shutdown_releases_a_waiter_without_polling_the_foreground_task() {
    let (store, requests) = fixture();
    let bridge = Arc::clone(&store.bridge);
    let worker = std::thread::spawn(move || {
        assert!(store.load("synthetic-a", Secret::SessionToken).is_err());
        assert!(
            store
                .store("synthetic-a", Secret::SessionToken, "synthetic")
                .is_err()
        );
    });
    let request = requests.recv_blocking().unwrap();
    bridge.stop();
    worker.join().unwrap();
    // A late native completion cannot reopen the bridge or block shutdown.
    request.complete(Ok(Some("synthetic-late".to_owned())));
    assert!(requests.is_closed());
}

#[test]
fn synchronous_calls_on_the_gpui_thread_are_rejected_without_enqueueing() {
    let (store, requests) = fixture();
    assert!(store.load("synthetic-a", Secret::SessionToken).is_err());
    assert!(
        store
            .store("synthetic-a", Secret::SessionToken, "synthetic")
            .is_err()
    );
    assert!(store.remove("synthetic-a", Secret::SessionToken).is_err());
    assert!(requests.try_recv().is_err());
}

#[test]
fn account_replacement_overwrites_the_same_native_key_and_memory_value() {
    let (store, requests) = fixture();
    let worker = std::thread::spawn(move || {
        for value in ["synthetic-old-account", "synthetic-new-account"] {
            store
                .store("synthetic-a", Secret::SessionToken, value)
                .unwrap();
            assert_eq!(
                store
                    .load("synthetic-a", Secret::SessionToken)
                    .unwrap()
                    .as_deref(),
                Some(value)
            );
        }
    });
    let first = requests.recv_blocking().unwrap();
    let key = first.key.clone();
    first.complete(Ok(None));
    let second = requests.recv_blocking().unwrap();
    assert_eq!(key, second.key);
    assert!(
        matches!(&second.operation, Operation::Write(value) if value == "synthetic-new-account")
    );
    second.complete(Ok(None));
    worker.join().unwrap();
}

#[test]
fn malformed_native_entries_are_rejected_without_backend_error_text() {
    assert_eq!(super::decode(None).unwrap(), None);
    assert_eq!(
        super::decode(Some((super::USERNAME.into(), b"synthetic".to_vec())))
            .unwrap()
            .as_deref(),
        Some("synthetic")
    );
    assert_eq!(
        super::decode(Some(("unexpected".into(), vec![]))),
        Err(CredentialError::InvalidEntry)
    );
    assert_eq!(
        super::decode(Some((super::USERNAME.into(), vec![0xff]))),
        Err(CredentialError::InvalidEntry)
    );
}

fn unavailable_native(
    _: &super::Request,
    _: &gpui::App,
) -> gpui::Task<Result<Option<String>, CredentialError>> {
    gpui::Task::ready(Err(CredentialError::Unavailable))
}

#[gpui_kit::test]
async fn failures_reach_the_foreground_with_original_request_identity(
    cx: &mut gpui::TestAppContext,
) {
    cx.executor().allow_parking();
    let foreground = std::thread::current().id();
    let (notices, received) = async_channel::bounded(3);
    let service = cx.update(|cx| {
        super::CredentialService::with_native(
            cx,
            move |failure, _| {
                assert_eq!(std::thread::current().id(), foreground);
                notices
                    .try_send((
                        failure.server().to_owned(),
                        failure.secret(),
                        failure.operation(),
                        failure.error(),
                    ))
                    .unwrap();
            },
            unavailable_native,
        )
    });
    let store = service.store();
    let (done, finished) = async_channel::bounded(1);
    let worker = std::thread::spawn(move || {
        assert!(store.load("synthetic-a", Secret::SessionToken).is_err());
        assert!(
            store
                .store("synthetic-a", Secret::ServerPassword, "synthetic")
                .is_err()
        );
        assert!(store.remove("synthetic-a", Secret::ServerPassword).is_err());
        done.send_blocking(()).unwrap();
    });
    finished.recv().await.unwrap();
    worker.join().unwrap();
    for (secret, operation) in [
        (Secret::SessionToken, super::CredentialOperation::Read),
        (Secret::ServerPassword, super::CredentialOperation::Write),
        (Secret::ServerPassword, super::CredentialOperation::Delete),
    ] {
        assert_eq!(
            received.try_recv().unwrap(),
            (
                "synthetic-a".to_owned(),
                secret,
                operation,
                CredentialError::Unavailable
            )
        );
    }
}

struct NativeEntered(async_channel::Sender<()>);
impl gpui::Global for NativeEntered {}

fn held_native(
    _: &super::Request,
    cx: &gpui::App,
) -> gpui::Task<Result<Option<String>, CredentialError>> {
    use gpui::AppContext as _;
    cx.global::<NativeEntered>().0.try_send(()).unwrap();
    cx.background_spawn(std::future::pending())
}

#[gpui_kit::test]
async fn production_quit_releases_worker_before_native_task_completes(
    cx: &mut gpui::TestAppContext,
) {
    cx.executor().allow_parking();
    cx.update(crate::session::install);
    let (entered, started) = async_channel::bounded(1);
    let service = cx.update(|cx| {
        cx.set_global(NativeEntered(entered));
        super::CredentialService::with_native(
            cx,
            |_, _| panic!("canceled native requests must not notify"),
            held_native,
        )
    });
    let store = service.store();
    let worker = std::thread::spawn(move || {
        assert!(store.load("synthetic-a", Secret::SessionToken).is_err());
    });
    started.recv().await.unwrap();
    cx.update(crate::session::quit_after_shutdown);
    // Deliberately join here without giving the foreground executor another turn.
    // The production quit fence must synchronously close the pending reply.
    worker.join().unwrap();
}

#[gpui_kit::test]
async fn headless_gpui_dispatch_serves_core_without_a_window(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    let service = cx.update(|cx| {
        super::CredentialService::new(cx, |_, _| panic!("test platform should succeed"))
    });
    let store = service.store();
    let (done, finished) = async_channel::bounded(1);
    let worker = std::thread::spawn(move || {
        assert_eq!(
            store.load("synthetic-a", Secret::SessionToken).unwrap(),
            None
        );
        store
            .store("synthetic-a", Secret::SessionToken, "synthetic")
            .unwrap();
        assert_eq!(
            store
                .load("synthetic-a", Secret::SessionToken)
                .unwrap()
                .as_deref(),
            Some("synthetic")
        );
        store.forget_server("synthetic-a").unwrap();
        assert_eq!(
            store.load("synthetic-a", Secret::SessionToken).unwrap(),
            None
        );
        done.send_blocking(()).unwrap();
    });
    finished.recv().await.unwrap();
    worker.join().unwrap();
}
