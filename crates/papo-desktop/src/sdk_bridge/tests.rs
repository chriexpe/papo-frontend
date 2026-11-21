//! The bridge against a real SDK client on a temporary profile, with the
//! SDK's scripted credential store and no network. Callbacks go to a mailbox
//! instead of Rune's app handle, and the test applies them to a headless
//! `Runtime` with `update_if`, which is what the app handle does on the UI
//! thread.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use papo_sdk::secret::test_store::ScriptedSecretStore;
use papo_sdk::{Client, ClientConfig, DataRoot};
use rune::{Cx, El, Env, Runtime, col};
use tokio::sync::oneshot;

use super::{Bridge, BridgeHost, Callback, OpenFailure, Post, Profile, RequestSlot, Sdk};

/// A guard against a hung bridge, not a timing assumption: every wait below
/// ends as soon as its callback arrives.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// Callbacks posted by the bridge, in posting order.
struct Mailbox(mpsc::Sender<Callback<Probe>>);

impl Post<Probe> for Mailbox {
    fn post(&self, callback: Callback<Probe>) {
        let _ = self.0.send(callback);
    }
}

/// The smallest UI state a bridge serves.
#[derive(Default)]
struct Probe {
    sdk: Option<Sdk<Probe>>,
    failure: Option<OpenFailure>,
    lookup: RequestSlot,
    answer: Option<&'static str>,
}

impl BridgeHost for Probe {
    fn sdk_opened(&mut self, sdk: Sdk<Self>) {
        self.sdk = Some(sdk);
    }

    fn sdk_failed(&mut self, failure: OpenFailure) {
        self.failure = Some(failure);
    }
}

fn view(_: &Probe, _: &mut Cx<Probe>) -> El {
    col()
}

struct Harness {
    bridge: Bridge,
    rt: Runtime<Probe>,
    inbox: mpsc::Receiver<Callback<Probe>>,
}

impl Harness {
    /// Applies the next posted callback as the app handle would, returning
    /// whether it asked for a redraw.
    fn deliver_next(&mut self) -> bool {
        let callback = self.inbox.recv_timeout(HANG_GUARD).expect("a callback");
        let mut redraw = false;
        self.rt.update_if(|state, tx| {
            redraw = callback(state, tx);
            redraw
        });
        redraw
    }
}

fn profile(root: &tempfile::TempDir) -> Profile {
    Profile {
        root: DataRoot::new(root.path()),
        secrets: Arc::new(ScriptedSecretStore::default()),
        config: ClientConfig::default(),
    }
}

fn start(root: &tempfile::TempDir) -> Harness {
    let (outbox, inbox) = mpsc::channel();
    let bridge = Bridge::start(profile(root), Arc::new(Mailbox(outbox))).unwrap();
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let rt = Runtime::new(Probe::default(), view, (100.0, 100.0), env);
    Harness { bridge, rt, inbox }
}

fn open(root: &tempfile::TempDir) -> Harness {
    let mut harness = start(root);
    harness.deliver_next();
    assert!(harness.rt.state.sdk.is_some(), "the client opened");
    harness
}

#[test]
fn a_superseded_request_result_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = open(&root);
    let sdk = harness.rt.state.sdk.clone().unwrap();

    // The first lookup is slow; the user changes their mind before it ends.
    let (release_first, first_released) = oneshot::channel::<()>();
    let first = harness.rt.state.lookup.issue();
    sdk.request(
        first,
        |state: &Probe| &state.lookup,
        move |_client| async move {
            let _ = first_released.await;
            "first"
        },
        |state: &mut Probe, answer| {
            state.answer = Some(answer);
            true
        },
    );
    let second = harness.rt.state.lookup.issue();
    sdk.request(
        second,
        |state: &Probe| &state.lookup,
        |_client| async { "second" },
        |state: &mut Probe, answer| {
            state.answer = Some(answer);
            true
        },
    );

    assert!(harness.deliver_next(), "the current result redraws");
    assert_eq!(harness.rt.state.answer, Some("second"));

    release_first.send(()).unwrap();
    assert!(!harness.deliver_next(), "a stale result does not redraw");
    assert_eq!(
        harness.rt.state.answer,
        Some("second"),
        "the stale result was dropped"
    );
    harness.bridge.shutdown();
}

#[test]
fn a_result_that_arrives_after_it_was_superseded_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = open(&root);
    let sdk = harness.rt.state.sdk.clone().unwrap();
    let ticket = harness.rt.state.lookup.issue();
    sdk.request(
        ticket,
        |state: &Probe| &state.lookup,
        |_client| async { "late" },
        |state: &mut Probe, answer| {
            state.answer = Some(answer);
            true
        },
    );
    // The user moves on while the answer is on its way.
    harness.rt.state.lookup.issue();
    assert!(!harness.deliver_next());
    assert_eq!(harness.rt.state.answer, None);
    harness.bridge.shutdown();
}

#[test]
fn work_is_given_the_open_client() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = open(&root);
    let sdk = harness.rt.state.sdk.clone().unwrap();
    let ticket = harness.rt.state.lookup.issue();
    sdk.request(
        ticket,
        |state: &Probe| &state.lookup,
        |client| async move {
            let servers = client.subscribe_registry().borrow().servers.len();
            if servers == 0 {
                "no servers"
            } else {
                "servers"
            }
        },
        |state: &mut Probe, answer| {
            state.answer = Some(answer);
            true
        },
    );
    harness.deliver_next();
    assert_eq!(harness.rt.state.answer, Some("no servers"));
    harness.bridge.shutdown();
}

#[test]
fn the_exit_fence_releases_the_profile() {
    let root = tempfile::tempdir().unwrap();
    let harness = open(&root);
    harness.bridge.shutdown();
    let mut reopened = Client::open(
        DataRoot::new(root.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .expect("the fence shut the first client down and released its lock");
    reopened.shutdown().unwrap();
}

#[test]
fn shutting_down_right_after_start_releases_the_profile() {
    let root = tempfile::tempdir().unwrap();
    let harness = start(&root);
    // The window closes at once: the bridge may still be opening the
    // profile, or may just have opened it. Either way the fence ends with
    // the client shut down.
    harness.bridge.shutdown();
    let mut reopened = Client::open(
        DataRoot::new(root.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    reopened.shutdown().unwrap();
}

#[test]
fn a_second_owner_of_the_profile_is_told_so() {
    let root = tempfile::tempdir().unwrap();
    let first = open(&root);
    let mut second = start(&root);
    second.deliver_next();
    assert_eq!(second.rt.state.failure, Some(OpenFailure::ProfileInUse));
    assert!(second.rt.state.sdk.is_none());
    second.bridge.shutdown();
    first.bridge.shutdown();
}

#[test]
fn requests_after_the_fence_are_dropped() {
    let root = tempfile::tempdir().unwrap();
    let mut harness = open(&root);
    let sdk = harness.rt.state.sdk.clone().unwrap();
    harness.bridge.shutdown();
    let ticket = harness.rt.state.lookup.issue();
    sdk.request(
        ticket,
        |state: &Probe| &state.lookup,
        |_client| async { "after exit" },
        |state: &mut Probe, answer| {
            state.answer = Some(answer);
            true
        },
    );
    // The bridge's queue closed with the fence, so the request was dropped
    // when it was sent and nothing can be posted for it.
    assert!(
        harness.inbox.try_recv().is_err(),
        "nothing ran after the fence"
    );
}

#[test]
fn work_sent_to_finish_lands_before_the_fence_ends() {
    let root = tempfile::tempdir().unwrap();
    let harness = open(&root);
    let sdk = harness.rt.state.sdk.clone().unwrap();
    let landed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&landed);
    // Sent right before the window closes: still queued, or just started,
    // when the fence begins.
    sdk.finish(move |_client| async move {
        tokio::task::yield_now().await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    harness.bridge.shutdown();
    assert!(landed.load(std::sync::atomic::Ordering::SeqCst));
}
