//! Verifies write-once native media engine factory registration in an isolated process binary:
//! first registration succeeds, second registration fails with `AlreadyRegistered`, and `open`
//! wires the registered factory into the single process-owned client (`join_call` hits it).

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use papo_ffi::{
    FfiClient, FfiClientConfig, FfiConnectionListener, FfiConnectionState, FfiConnectionStatus,
    FfiSecretKey, FfiSecretRead, FfiSecretStore, FfiSecretStoreError,
    MediaFactoryRegistrationError, register_media_engine_factory_once,
};
use papo_sdk::FakeMediaFactory;
use papo_test_server::{Config, TestServer};

#[derive(Default)]
struct InMemoryStore {
    entries: Mutex<BTreeMap<(String, String, u64), Vec<u8>>>,
}

impl FfiSecretStore for InMemoryStore {
    fn read(&self, key: FfiSecretKey) -> Result<FfiSecretRead, FfiSecretStoreError> {
        let value = self
            .entries
            .lock()
            .unwrap()
            .get(&(key.account.server_id, key.account.user_id, key.generation))
            .cloned();
        Ok(FfiSecretRead {
            value,
            settled: true,
        })
    }
    fn write(&self, key: FfiSecretKey, value: Vec<u8>) -> Result<(), FfiSecretStoreError> {
        self.entries.lock().unwrap().insert(
            (key.account.server_id, key.account.user_id, key.generation),
            value,
        );
        Ok(())
    }
    fn delete(&self, key: FfiSecretKey) -> Result<(), FfiSecretStoreError> {
        self.entries.lock().unwrap().remove(&(
            key.account.server_id,
            key.account.user_id,
            key.generation,
        ));
        Ok(())
    }
}

struct Online(mpsc::Sender<()>);

impl FfiConnectionListener for Online {
    fn on_snapshot(&self, s: FfiConnectionStatus) {
        if matches!(s.state, FfiConnectionState::Online) {
            let _ = self.0.send(());
        }
    }
    fn on_update(&self, s: FfiConnectionStatus) {
        if matches!(s.state, FfiConnectionState::Online) {
            let _ = self.0.send(());
        }
    }
    fn on_closed(&self) {}
}

#[test]
fn register_media_factory_once_rejects_second_and_applies_on_open() {
    let factory = FakeMediaFactory::new();
    let engine1: Arc<dyn papo_sdk::MediaEngineFactory> = Arc::new(factory.clone());
    assert_eq!(register_media_engine_factory_once(engine1), Ok(()));
    assert_eq!(
        register_media_engine_factory_once(Arc::new(FakeMediaFactory::new())),
        Err(MediaFactoryRegistrationError::AlreadyRegistered)
    );

    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let call_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let server = server_rt
        .block_on(TestServer::start(Config::default()))
        .unwrap();
    let voice = server_rt
        .block_on(server.create_voice_channel("voice-once"))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = FfiClientConfig {
        data_root_path: dir.path().to_str().unwrap().to_owned(),
        worker_threads: 2,
        credential_timeout_ms: 1_000,
        request_timeout_ms: 3_000,
        disk_budget_bytes: 16 * 1024 * 1024,
        call_join_timeout_ms: 1_000,
        call_drain_timeout_ms: 500,
        giphy_api_key: None,
        insecure_loopback: true,
    };
    let client = FfiClient::open(cfg, Arc::new(InMemoryStore::default())).unwrap();

    let (tx, rx) = mpsc::channel();
    let account = call_rt.block_on(async {
        let added = client.add_server(server.base_url(), 100).await.unwrap();
        let auth = client.server_auth(added.server_id).unwrap();
        auth.register("alice".into(), "Aabcdefghijklmnop!".into())
            .await
            .unwrap();
        auth.login("alice".into(), "Aabcdefghijklmnop!".into(), 100)
            .await
            .unwrap()
    });
    let sub = client
        .subscribe_connection(account.clone(), Arc::new(Online(tx)))
        .unwrap();
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    sub.dispose();

    // Wait briefly for channel resource admission, then join and confirm registered factory starts.
    let ch_id = voice.id.unwrap();
    for _ in 0..50 {
        if client.join_call(account.clone(), ch_id.clone()).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    server_rt.block_on(factory.wait_starts(1));
    assert_eq!(factory.start_count(), 1);

    let report = client.shutdown(500).unwrap();
    assert!(report.reads_drained);
    server_rt.block_on(server.shutdown()).unwrap();
}
