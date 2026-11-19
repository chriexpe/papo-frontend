//! The keyring worker's contract with the SDK, against an in-memory backend:
//! calls run in submission order, a dropped call still finishes, nothing
//! starts before its first poll, and entries are scoped by the whole key.
//! The real Secret Service round trip is the ignored test at the end.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use papo_sdk::secret::{
    CredentialGeneration, SecretKey, SecretStore, SecretStoreError, SecretValue,
};
use papo_sdk::{AccountId, ServerAddress, Uuid};
use tokio::sync::Semaphore;

use super::{Backend, Keyring, SecretService, secret_service};

/// What the fake keyring holds and how often it was asked.
#[derive(Default)]
struct Shelf {
    entries: HashMap<SecretKey, Vec<u8>>,
    writes: usize,
}

/// An in-memory backend. A write says it started on `started`, then waits
/// for one permit from `gate`, so a test decides when it finishes.
struct Fake {
    shelf: Arc<Mutex<Shelf>>,
    gate: Arc<Semaphore>,
    started: std::sync::mpsc::Sender<()>,
}

impl Backend for Fake {
    fn read(
        &mut self,
        key: SecretKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, SecretStoreError>> {
        std::future::ready(Ok(self.shelf.lock().unwrap().entries.get(&key).cloned()))
    }

    async fn write(&mut self, key: SecretKey, value: SecretValue) -> Result<(), SecretStoreError> {
        let _ = self.started.send(());
        self.gate.acquire().await.unwrap().forget();
        let mut held = self.shelf.lock().unwrap();
        held.writes += 1;
        held.entries.insert(key, value.expose().to_vec());
        Ok(())
    }

    fn delete(&mut self, key: SecretKey) -> impl Future<Output = Result<(), SecretStoreError>> {
        self.shelf.lock().unwrap().entries.remove(&key);
        std::future::ready(Ok(()))
    }
}

struct Harness {
    keyring: Keyring,
    shelf: Arc<Mutex<Shelf>>,
    gate: Arc<Semaphore>,
    started: std::sync::mpsc::Receiver<()>,
}

/// A keyring whose writes finish only when the test adds permits to `gate`.
fn harness() -> Harness {
    let shelf = Arc::new(Mutex::new(Shelf::default()));
    let gate = Arc::new(Semaphore::new(0));
    let (started_tx, started) = std::sync::mpsc::channel();
    let backend = Fake {
        shelf: Arc::clone(&shelf),
        gate: Arc::clone(&gate),
        started: started_tx,
    };
    let keyring = Keyring::start(move || backend).unwrap();
    Harness {
        keyring,
        shelf,
        gate,
        started,
    }
}

fn key(user: u128, generation: u64) -> SecretKey {
    let server = ServerAddress::parse("https://chat.example.invalid")
        .unwrap()
        .id();
    SecretKey::new(
        AccountId::new(server, Uuid::from_u128(user)),
        CredentialGeneration::new(generation),
    )
}

fn value(bytes: &[u8]) -> SecretValue {
    SecretValue::new(bytes.to_vec())
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Polls `future` once, so its request is queued, and gives it back.
fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    let mut context = Context::from_waker(Waker::noop());
    std::pin::Pin::new(future).poll(&mut context)
}

#[test]
fn a_dropped_write_still_finishes_before_the_next_read() {
    let harness = harness();
    let mut write = harness.keyring.write(key(1, 1), value(b"token"));
    assert!(
        poll_once(&mut write).is_pending(),
        "the write waits for its gate"
    );
    harness.started.recv().unwrap();
    // The SDK gave up on the write (a timeout): its future is dropped, the
    // OS call is not.
    drop(write);
    harness.gate.add_permits(1);
    block_on(harness.keyring.read(key(1, 1))).unwrap();
    let shelf = harness.shelf.lock().unwrap();
    assert_eq!(shelf.writes, 1, "the dropped write ran to its end");
    assert_eq!(
        shelf.entries.get(&key(1, 1)).map(Vec::as_slice),
        Some(&b"token"[..])
    );
}

#[test]
fn a_queued_call_whose_caller_gave_up_never_runs() {
    let harness = harness();
    let mut running = harness.keyring.write(key(1, 1), value(b"first"));
    assert!(poll_once(&mut running).is_pending());
    harness.started.recv().unwrap();
    let mut queued = harness.keyring.write(key(1, 1), value(b"abandoned"));
    assert!(poll_once(&mut queued).is_pending());
    // The SDK's deadline passed while the call waited behind the first one.
    drop(queued);
    harness.gate.add_permits(2);
    block_on(running).unwrap();
    block_on(harness.keyring.read(key(1, 1))).unwrap();
    let shelf = harness.shelf.lock().unwrap();
    assert_eq!(shelf.writes, 1, "the abandoned write was skipped");
    assert_eq!(shelf.entries[&key(1, 1)], b"first");
}

#[test]
fn reads_wait_behind_every_earlier_write() {
    let harness = harness();
    let mut first = harness.keyring.write(key(1, 1), value(b"old"));
    let mut second = harness.keyring.write(key(1, 1), value(b"new"));
    assert!(poll_once(&mut first).is_pending());
    assert!(poll_once(&mut second).is_pending());
    let mut read = harness.keyring.read(key(1, 1));
    assert!(
        poll_once(&mut read).is_pending(),
        "the read is queued behind both writes"
    );
    assert_eq!(harness.shelf.lock().unwrap().writes, 0);
    harness.gate.add_permits(2);
    block_on(read).unwrap();
    assert_eq!(
        harness.shelf.lock().unwrap().writes,
        2,
        "both writes finished before the read answered"
    );
    block_on(first).unwrap();
    block_on(second).unwrap();
    assert_eq!(
        harness.shelf.lock().unwrap().entries[&key(1, 1)],
        b"new",
        "writes land in submission order"
    );
}

#[test]
fn nothing_reaches_the_keyring_before_the_first_poll() {
    let harness = harness();
    harness.gate.add_permits(1);
    let unpolled = harness.keyring.write(key(1, 1), value(b"token"));
    drop(unpolled);
    // A later call that has answered proves the worker has seen everything
    // queued before it.
    block_on(harness.keyring.read(key(1, 1))).unwrap();
    assert_eq!(harness.shelf.lock().unwrap().writes, 0);
}

#[test]
fn a_closed_keyring_refuses_calls() {
    let harness = harness();
    harness.keyring.close();
    let result = block_on(harness.keyring.delete(key(1, 1)));
    assert_eq!(result.err(), Some(SecretStoreError::Unavailable));
}

#[test]
fn every_part_of_the_key_names_a_different_entry() {
    let base = secret_service::attributes(key(1, 1));
    assert_ne!(base, secret_service::attributes(key(2, 1)), "another user");
    assert_ne!(
        base,
        secret_service::attributes(key(1, 2)),
        "another generation"
    );
    let other_server = SecretKey::new(
        AccountId::new(
            ServerAddress::parse("https://other.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        ),
        CredentialGeneration::new(1),
    );
    assert_ne!(
        base,
        secret_service::attributes(other_server),
        "another server"
    );
}

/// Writes, reads back and deletes one entry in the session's real keyring,
/// through the backend (the SDK's `SecretRead` does not show its value).
/// Needs a Secret Service, or the Secret portal inside Flatpak, so it runs
/// only on request: `cargo test -p papo-desktop -- --ignored real_keyring`.
#[test]
#[ignore = "needs the session's Secret Service"]
fn the_real_keyring_round_trips_an_entry() {
    let mut service = SecretService::default();
    // A user id no account has, so no real entry is touched.
    let key = key(u128::MAX, u64::MAX);
    block_on(async {
        service.write(key, value(b"round trip")).await.unwrap();
        assert_eq!(
            service.read(key).await.unwrap().as_deref(),
            Some(&b"round trip"[..])
        );
        service.write(key, value(b"replaced")).await.unwrap();
        assert_eq!(
            service.read(key).await.unwrap().as_deref(),
            Some(&b"replaced"[..])
        );
        service.delete(key).await.unwrap();
        assert_eq!(service.read(key).await.unwrap(), None);
        service.delete(key).await.unwrap();
    });
}
