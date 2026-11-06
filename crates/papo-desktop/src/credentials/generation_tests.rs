//! Generation regressions with controlled native completions, never OS storage.

use super::{CredentialError, CredentialFailure, CredentialService, Request};
use gpui::{App, AppContext as _, Global, Task};
use papo_core::storage::{Secret, SecretStore as _};
use std::sync::Arc;

type NativeResult = Result<Option<String>, CredentialError>;

struct ControlledNative {
    entered: async_channel::Sender<()>,
    completions: async_channel::Receiver<NativeResult>,
}
impl Global for ControlledNative {}

fn controlled_native(_: &Request, cx: &App) -> Task<NativeResult> {
    let native = cx.global::<ControlledNative>();
    native.entered.try_send(()).unwrap();
    let completions = native.completions.clone();
    cx.background_spawn(async move { completions.recv().await.unwrap() })
}

struct Harness {
    service: CredentialService,
    started: async_channel::Receiver<()>,
    complete: async_channel::Sender<NativeResult>,
    failures: async_channel::Receiver<CredentialFailure>,
}

fn harness(cx: &mut gpui::TestAppContext) -> Harness {
    cx.executor().allow_parking();
    let (entered, started) = async_channel::bounded(4);
    let (complete, completions) = async_channel::bounded(4);
    let (notices, failures) = async_channel::bounded(4);
    let service = cx.update(|cx| {
        cx.set_global(ControlledNative {
            entered,
            completions,
        });
        CredentialService::with_native(
            cx,
            move |failure, _| notices.try_send(failure).unwrap(),
            controlled_native,
        )
    });
    Harness {
        service,
        started,
        complete,
        failures,
    }
}

fn worker<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> (std::thread::JoinHandle<()>, async_channel::Receiver<T>) {
    let (reply, finished) = async_channel::bounded(1);
    let worker = std::thread::spawn(move || reply.send_blocking(work()).unwrap());
    (worker, finished)
}

#[gpui_kit::test]
async fn replaced_account_native_failure_does_not_reach_the_callback(
    cx: &mut gpui::TestAppContext,
) {
    let mut probe = harness(cx);
    let old = probe.service.replace_store("synthetic-a");
    let (old_worker, old_result) = worker(move || old.load("synthetic-a", Secret::SessionToken));
    probe.started.recv().await.unwrap();
    let replacement = probe.service.replace_store("synthetic-a");
    probe
        .complete
        .try_send(Err(CredentialError::Unavailable))
        .unwrap();
    assert!(old_result.recv().await.unwrap().is_err());
    old_worker.join().unwrap();
    assert!(
        probe.failures.try_recv().is_err(),
        "old-account failure was delivered"
    );

    let (new_worker, new_result) =
        worker(move || replacement.load("synthetic-a", Secret::SessionToken));
    probe.started.recv().await.unwrap();
    probe
        .complete
        .try_send(Err(CredentialError::Unavailable))
        .unwrap();
    assert!(new_result.recv().await.unwrap().is_err());
    new_worker.join().unwrap();
    let failure = probe.failures.recv().await.unwrap();
    assert!(failure.is_current());
    assert_eq!(failure.server(), "synthetic-a");
}

#[gpui_kit::test]
async fn late_successful_read_is_not_returned_or_cached_after_replacement(
    cx: &mut gpui::TestAppContext,
) {
    let mut probe = harness(cx);
    let old = probe.service.replace_store("synthetic-a");
    let (old_worker, old_result) = worker(move || old.load("synthetic-a", Secret::SessionToken));
    probe.started.recv().await.unwrap();
    let replacement = probe.service.replace_store("synthetic-a");
    probe
        .complete
        .try_send(Ok(Some("synthetic-old".into())))
        .unwrap();
    assert_eq!(
        old_result.recv().await.unwrap().unwrap_err().to_string(),
        CredentialError::Stale.to_string()
    );
    old_worker.join().unwrap();

    let (new_worker, new_result) =
        worker(move || replacement.load("synthetic-a", Secret::SessionToken));
    // A new native read proves the old completion did not populate the overlay.
    probe.started.recv().await.unwrap();
    probe
        .complete
        .try_send(Ok(Some("synthetic-new".into())))
        .unwrap();
    assert_eq!(
        new_result.recv().await.unwrap().unwrap().as_deref(),
        Some("synthetic-new")
    );
    new_worker.join().unwrap();
    assert!(probe.failures.try_recv().is_err());
}

#[gpui_kit::test]
async fn revoked_native_mutations_do_not_return_success(cx: &mut gpui::TestAppContext) {
    let mut probe = harness(cx);
    for operation in [
        super::CredentialOperation::Write,
        super::CredentialOperation::Delete,
    ] {
        let store = probe.service.replace_store("synthetic-a");
        let (thread, result) = worker(move || match operation {
            super::CredentialOperation::Write => {
                store.store("synthetic-a", Secret::SessionToken, "synthetic")
            }
            super::CredentialOperation::Delete => store.remove("synthetic-a", Secret::SessionToken),
            super::CredentialOperation::Read => unreachable!(),
        });
        probe.started.recv().await.unwrap();
        probe.service.invalidate("synthetic-a");
        probe.complete.try_send(Ok(None)).unwrap();
        assert_eq!(
            result.recv().await.unwrap().unwrap_err().to_string(),
            CredentialError::Stale.to_string()
        );
        thread.join().unwrap();
    }
    assert!(probe.failures.try_recv().is_err());
}

#[gpui_kit::test]
async fn wrong_server_calls_and_replaced_clones_cannot_mutate_or_enqueue(
    cx: &mut gpui::TestAppContext,
) {
    let mut probe = harness(cx);
    let old = probe.service.replace_store("synthetic-a");
    let old_clone = Arc::clone(&old);
    let other = probe.service.replace_store("synthetic-b");
    let _replacement = probe.service.replace_store("synthetic-a");
    let (thread, result) = worker(move || {
        for (store, server, expected) in [
            (old, "synthetic-a", CredentialError::Stale),
            (old_clone, "synthetic-a", CredentialError::Stale),
            (
                Arc::clone(&other),
                "synthetic-a",
                CredentialError::WrongServer,
            ),
        ] {
            assert_eq!(
                store
                    .load(server, Secret::SessionToken)
                    .unwrap_err()
                    .to_string(),
                expected.to_string()
            );
            assert_eq!(
                store
                    .store(server, Secret::SessionToken, "synthetic")
                    .unwrap_err()
                    .to_string(),
                expected.to_string()
            );
            assert_eq!(
                store
                    .remove(server, Secret::SessionToken)
                    .unwrap_err()
                    .to_string(),
                expected.to_string()
            );
            assert_eq!(
                store.forget_server(server).unwrap_err().to_string(),
                expected.to_string()
            );
        }
        other.load("synthetic-b", Secret::SessionToken)
    });
    probe.started.recv().await.unwrap();
    // Only the still-current other server may reach native storage.
    probe
        .complete
        .try_send(Ok(Some("synthetic-other".into())))
        .unwrap();
    assert_eq!(
        result.recv().await.unwrap().unwrap().as_deref(),
        Some("synthetic-other")
    );
    thread.join().unwrap();
    assert!(probe.started.try_recv().is_err());
    assert!(probe.failures.try_recv().is_err());
}

#[gpui_kit::test]
async fn failed_delete_tombstone_survives_a_new_incarnation(cx: &mut gpui::TestAppContext) {
    let mut probe = harness(cx);
    let store = probe.service.replace_store("synthetic-a");
    let (thread, result) = worker(move || store.remove("synthetic-a", Secret::SessionToken));
    probe.started.recv().await.unwrap();
    probe
        .complete
        .try_send(Err(CredentialError::Unavailable))
        .unwrap();
    assert!(result.recv().await.unwrap().is_err());
    thread.join().unwrap();
    let failure = probe.failures.recv().await.unwrap();
    let replacement = probe.service.replace_store("synthetic-a");
    assert!(!failure.is_current());
    let (thread, result) = worker(move || replacement.load("synthetic-a", Secret::SessionToken));
    assert_eq!(result.recv().await.unwrap().unwrap(), None);
    thread.join().unwrap();
    assert!(probe.started.try_recv().is_err());
}

#[gpui_kit::test]
async fn retained_failures_expire_on_replacement_invalidation_stop_and_drop(
    cx: &mut gpui::TestAppContext,
) {
    for transition in 0..4 {
        let mut probe = harness(cx);
        let store = probe.service.replace_store("synthetic-a");
        let (thread, result) = worker(move || store.load("synthetic-a", Secret::SessionToken));
        probe.started.recv().await.unwrap();
        probe
            .complete
            .try_send(Err(CredentialError::Unavailable))
            .unwrap();
        assert!(result.recv().await.unwrap().is_err());
        thread.join().unwrap();
        let failure = probe.failures.recv().await.unwrap();
        assert!(failure.is_current());
        match transition {
            0 => {
                let _replacement = probe.service.replace_store("synthetic-a");
            }
            1 => probe.service.invalidate("synthetic-a"),
            2 => probe.service.stop(),
            _ => drop(probe.service),
        }
        assert!(!failure.is_current());
    }
}

#[gpui_kit::test]
async fn production_quit_expires_delivered_failures_and_new_handles(cx: &mut gpui::TestAppContext) {
    cx.update(crate::session::install);
    let mut probe = harness(cx);
    let store = probe.service.replace_store("synthetic-a");
    let (thread, result) = worker(move || store.load("synthetic-a", Secret::SessionToken));
    probe.started.recv().await.unwrap();
    probe
        .complete
        .try_send(Err(CredentialError::Unavailable))
        .unwrap();
    assert!(result.recv().await.unwrap().is_err());
    thread.join().unwrap();
    let failure = probe.failures.recv().await.unwrap();
    assert!(failure.is_current());
    cx.update(crate::session::quit_after_shutdown);
    assert!(!failure.is_current());
    let store = probe.service.replace_store("synthetic-a");
    let thread = std::thread::spawn(move || {
        assert!(
            store
                .store("synthetic-a", Secret::SessionToken, "synthetic")
                .is_err()
        );
    });
    thread.join().unwrap();
    assert!(probe.started.try_recv().is_err());
}

#[gpui_kit::test]
async fn queued_request_is_rejected_before_native_dispatch(cx: &mut gpui::TestAppContext) {
    let (bridge, requests) = super::Bridge::new();
    let generation = Arc::new(super::Generation::new("synthetic-a", &bridge));
    let store = super::OsSecretStore::new(bridge, Arc::clone(&generation), Arc::default());
    let thread = std::thread::spawn(move || {
        assert_eq!(
            store
                .load("synthetic-a", Secret::SessionToken)
                .unwrap_err()
                .to_string(),
            CredentialError::Stale.to_string()
        );
    });
    let request = requests.recv_blocking().unwrap();
    generation.invalidate();
    // No ControlledNative global is installed: invoking it would panic. This
    // is the same foreground validation used by the production service loop.
    let result = cx
        .update(|cx| super::dispatch_current(&request, cx, controlled_native))
        .await;
    assert_eq!(result, Err(CredentialError::Stale));
    request.complete(result);
    thread.join().unwrap();
}
