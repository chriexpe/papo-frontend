//! Unit tests verifying `FfiSecretBridge` lazy polling, late timed-out writes, generation isolation,
//! and bridge replacement fencing (`settled = true` vs `settled = false` keeping `Uncertain`).

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use papo_sdk::{
    AccountId, ServerId, Uuid,
    secret::{
        CredentialGeneration, CredentialState, SecretError, SecretFailure, SecretKey,
        SecretStoreBoundary, SecretValue,
    },
};
use tokio_util::sync::CancellationToken;

use crate::{
    ids::FfiAccountId,
    secret::{FfiSecretBridge, FfiSecretKey, FfiSecretRead, FfiSecretStore, FfiSecretStoreError},
};

type KeyTuple = (String, String, u64);

fn to_tuple(key: &FfiSecretKey) -> KeyTuple {
    (
        key.account.server_id.clone(),
        key.account.user_id.clone(),
        key.generation,
    )
}

/// Process-like shared store backing two replaceable bridge/store handles with optional gate sharing.
#[derive(Default)]
struct SharedKeystoreBackend {
    gate: Mutex<()>,
    entries: Mutex<BTreeMap<KeyTuple, Vec<u8>>>,
}

/// Test `FfiSecretStore` simulating Keystore access with optional write delay and gate sharing.
struct ScriptedFfiStore {
    backend: Arc<SharedKeystoreBackend>,
    /// When false, reads do not acquire `backend.gate` and report `settled = false` (unfenced executor).
    shares_gate: bool,
    write_gate: Mutex<Option<mpsc::Receiver<()>>>,
    write_started: Arc<AtomicBool>,
    calls: Mutex<usize>,
}

impl ScriptedFfiStore {
    fn fenced(backend: Arc<SharedKeystoreBackend>) -> Self {
        Self {
            backend,
            shares_gate: true,
            write_gate: Mutex::new(None),
            write_started: Arc::new(AtomicBool::new(false)),
            calls: Mutex::new(0),
        }
    }

    fn unfenced(backend: Arc<SharedKeystoreBackend>) -> Self {
        Self {
            backend,
            shares_gate: false,
            write_gate: Mutex::new(None),
            write_started: Arc::new(AtomicBool::new(false)),
            calls: Mutex::new(0),
        }
    }

    fn block_next_write(&self) -> mpsc::Sender<()> {
        let (tx, rx) = mpsc::channel();
        *self
            .write_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(rx);
        tx
    }

    fn call_count(&self) -> usize {
        *self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl FfiSecretStore for ScriptedFfiStore {
    fn read(&self, key: FfiSecretKey) -> Result<FfiSecretRead, FfiSecretStoreError> {
        *self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        if self.shares_gate {
            let _gate = self
                .backend
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let value = self
                .backend
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&to_tuple(&key))
                .cloned();
            Ok(FfiSecretRead {
                value,
                settled: true,
            })
        } else {
            let value = self
                .backend
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&to_tuple(&key))
                .cloned();
            Ok(FfiSecretRead {
                value,
                settled: false,
            })
        }
    }

    fn write(&self, key: FfiSecretKey, value: Vec<u8>) -> Result<(), FfiSecretStoreError> {
        *self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let _gate = self
            .backend
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.write_started.store(true, Ordering::Release);
        if let Some(rx) = self
            .write_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = rx.recv();
        }
        self.backend
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(to_tuple(&key), value);
        Ok(())
    }

    fn delete(&self, key: FfiSecretKey) -> Result<(), FfiSecretStoreError> {
        *self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        let _gate = self
            .backend
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.backend
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&to_tuple(&key));
        Ok(())
    }
}

fn sample_key(generation: u64) -> SecretKey {
    let server: ServerId = "0000000000000000000000000000000a".parse().unwrap();
    let account = AccountId::new(server, Uuid::from_u128(20));
    SecretKey::new(account, CredentialGeneration::new(generation))
}

#[tokio::test]
async fn dropped_unpolled_future_does_not_touch_store() {
    let backend = Arc::new(SharedKeystoreBackend::default());
    let store = Arc::new(ScriptedFfiStore::fenced(Arc::clone(&backend)));
    let bridge = FfiSecretBridge::start(Arc::clone(&store) as Arc<dyn FfiSecretStore>).unwrap();
    let key = sample_key(1);
    {
        use papo_sdk::secret::SecretStore;
        let fut = bridge.write(key, SecretValue::new(vec![9, 9]));
        drop(fut);
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(store.call_count(), 0);
    assert!(
        backend
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[tokio::test]
async fn timed_out_write_recovers_via_fenced_read_without_touching_next_generation() {
    let backend = Arc::new(SharedKeystoreBackend::default());
    let store = Arc::new(ScriptedFfiStore::fenced(Arc::clone(&backend)));
    let bridge = FfiSecretBridge::start(Arc::clone(&store) as Arc<dyn FfiSecretStore>).unwrap();
    let boundary = SecretStoreBoundary::new(bridge, Duration::from_millis(30));
    let cancel = CancellationToken::new();

    let gen1 = sample_key(1);
    boundary
        .activate(gen1.account(), gen1.generation())
        .unwrap();
    let release = store.block_next_write();
    let err = boundary
        .write(gen1, SecretValue::new(vec![1, 2, 3]), &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, SecretError::Uncertain(SecretFailure::Timeout));
    assert_eq!(boundary.state(gen1).unwrap(), CredentialState::Uncertain);

    // Activate gen + 1 while gen1's OS write is still blocked; gen + 1 operates independently.
    let gen2 = sample_key(2);
    boundary
        .activate(gen2.account(), gen2.generation())
        .unwrap();
    // Fenced read/write on gen1's same bridge waits until release; verify gen1 key recovers on release.
    let boundary_recovery = {
        let store2 = Arc::new(ScriptedFfiStore::fenced(Arc::clone(&backend)));
        let bridge2 = FfiSecretBridge::start(store2).unwrap();
        SecretStoreBoundary::new(bridge2, Duration::from_secs(2))
    };
    boundary_recovery
        .activate(gen1.account(), gen1.generation())
        .unwrap();

    // Release delayed write from a background thread after 40ms so bridge2 fences behind it.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        let _ = release.send(());
    });

    let recovered = boundary_recovery
        .read(gen1, &cancel)
        .await
        .unwrap()
        .expect("late write committed before fenced read returned");
    assert_eq!(recovered.expose(), &[1, 2, 3]);
    assert_eq!(
        boundary_recovery.state(gen1).unwrap(),
        CredentialState::Ready
    );

    // Now operate gen + 1 cleanly and verify old key write never touched gen + 1.
    assert!(boundary.read(gen2, &cancel).await.unwrap().is_none());
    boundary
        .write(gen2, SecretValue::new(vec![7, 8]), &cancel)
        .await
        .unwrap();
    assert_eq!(
        boundary
            .read(gen2, &cancel)
            .await
            .unwrap()
            .unwrap()
            .expose(),
        &[7, 8]
    );
    let _ = FfiAccountId::from_sdk(gen2.account());
}

#[tokio::test]
async fn bridge_replacement_fences_only_when_sharing_process_gate() {
    let backend = Arc::new(SharedKeystoreBackend::default());
    let store1 = Arc::new(ScriptedFfiStore::fenced(Arc::clone(&backend)));
    let bridge1 = FfiSecretBridge::start(Arc::clone(&store1) as Arc<dyn FfiSecretStore>).unwrap();
    let boundary1 = SecretStoreBoundary::new(bridge1, Duration::from_millis(30));
    let cancel = CancellationToken::new();
    let key = sample_key(5);
    boundary1.activate(key.account(), key.generation()).unwrap();

    let release = store1.block_next_write();
    let err = boundary1
        .write(key, SecretValue::new(vec![42]), &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, SecretError::Uncertain(SecretFailure::Timeout));

    // Wait briefly so store1 holds `backend.gate`.
    while !store1.write_started.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Unfenced replacement bridge reports `settled = false`, so SDK keeps key `Uncertain`.
    let unfenced_store = Arc::new(ScriptedFfiStore::unfenced(Arc::clone(&backend)));
    let unfenced_bridge = FfiSecretBridge::start(unfenced_store).unwrap();
    let unfenced_boundary = SecretStoreBoundary::new(unfenced_bridge, Duration::from_secs(1));
    unfenced_boundary
        .activate(key.account(), key.generation())
        .unwrap();
    let Err(unfenced_err) = unfenced_boundary.read(key, &cancel).await else {
        panic!("unfenced read must never resolve uncertain state");
    };
    assert_eq!(
        unfenced_err,
        SecretError::Uncertain(SecretFailure::Unfenced)
    );
    assert_eq!(
        unfenced_boundary.state(key).unwrap(),
        CredentialState::Uncertain
    );

    // Shared-gate replacement bridge blocks behind older in-flight write and recovers cleanly.
    let fenced_store = Arc::new(ScriptedFfiStore::fenced(Arc::clone(&backend)));
    let fenced_bridge = FfiSecretBridge::start(fenced_store).unwrap();
    let fenced_boundary = SecretStoreBoundary::new(fenced_bridge, Duration::from_secs(2));
    fenced_boundary
        .activate(key.account(), key.generation())
        .unwrap();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        let _ = release.send(());
    });

    let read = fenced_boundary.read(key, &cancel).await.unwrap().unwrap();
    assert_eq!(read.expose(), &[42]);
    assert_eq!(fenced_boundary.state(key).unwrap(), CredentialState::Ready);
}
