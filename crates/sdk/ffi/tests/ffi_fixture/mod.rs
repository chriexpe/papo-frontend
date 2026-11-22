//! Shared loopback `TestServer` and off-Tokio `FfiClient` harness for FFI integration tests.
//! Keeps fixtures local to `tests/` without logging secrets, addresses, or message bodies.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use papo_ffi::{
    FfiAccountId, FfiClient, FfiClientConfig, FfiConnectionListener, FfiConnectionState,
    FfiConnectionStatus, FfiSecretKey, FfiSecretRead, FfiSecretStore, FfiSecretStoreError,
    FfiShutdownReport, FfiUtcOffsetProvider,
};
use papo_sdk::FakeMediaFactory;
use papo_test_server::{Config, TestServer};
use tokio::runtime::Runtime;

/// Key tuple identifying an account/generation entry without leaking raw bytes.
pub type KeyTuple = (String, String, u64);

fn key_tuple(key: &FfiSecretKey) -> KeyTuple {
    (
        key.account.server_id.clone(),
        key.account.user_id.clone(),
        key.generation,
    )
}

/// Deterministic in-memory `FfiSecretStore` supporting write holds and unfenced-read tests.
#[derive(Default)]
pub struct TestFfiSecretStore {
    gate: Mutex<()>,
    entries: Mutex<BTreeMap<KeyTuple, Vec<u8>>>,
    unfenced: AtomicBool,
}

impl TestFfiSecretStore {
    /// Toggles `settled` returned on subsequent reads.
    pub fn set_unfenced(&self, unfenced: bool) {
        self.unfenced.store(unfenced, Ordering::Release);
    }

    /// Snapshots stored byte length for diagnostics without logging secret bytes.
    pub fn entry_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl FfiSecretStore for TestFfiSecretStore {
    fn read(&self, key: FfiSecretKey) -> Result<FfiSecretRead, FfiSecretStoreError> {
        if self.unfenced.load(Ordering::Acquire) {
            let value = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key_tuple(&key))
                .cloned();
            return Ok(FfiSecretRead {
                value,
                settled: false,
            });
        }
        let _gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let value = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key_tuple(&key))
            .cloned();
        Ok(FfiSecretRead {
            value,
            settled: true,
        })
    }

    fn write(&self, key: FfiSecretKey, value: Vec<u8>) -> Result<(), FfiSecretStoreError> {
        let _gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key_tuple(&key), value);
        Ok(())
    }

    fn delete(&self, key: FfiSecretKey) -> Result<(), FfiSecretStoreError> {
        let _gate = self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key_tuple(&key));
        Ok(())
    }
}

/// Zero-offset UTC provider for deterministic timeline tests.
pub struct ZeroUtcOffset;

impl FfiUtcOffsetProvider for ZeroUtcOffset {
    fn offset_at_seconds_east(&self, _created_at_us: i64) -> i32 {
        0
    }
}

/// Complete loopback test harness holding `TestServer`, caller runtime, `FfiClient`, and temp dir.
pub struct FfiTestHarness {
    pub client: Option<Arc<FfiClient>>,
    pub store: Arc<TestFfiSecretStore>,
    pub server: Option<TestServer>,
    pub server_rt: Option<Runtime>,
    pub call_rt: Runtime,
    pub profile: tempfile::TempDir,
}

impl FfiTestHarness {
    /// Boots `TestServer` and opens `FfiClient` with an explicit optional `FakeMediaFactory`.
    pub fn open(config: Config, media: Option<FakeMediaFactory>) -> Self {
        let server_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("server runtime");
        let call_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("caller runtime");
        let server = server_rt
            .block_on(TestServer::start(config))
            .expect("test server starts");
        let profile = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(TestFfiSecretStore::default());
        let cfg = client_config(profile.path().to_str().expect("utf8 path"));
        let engine = media.map(|m| Arc::new(m) as Arc<dyn papo_sdk::MediaEngineFactory>);
        let client = FfiClient::open_with_media_engine(
            cfg,
            Arc::clone(&store) as Arc<dyn FfiSecretStore>,
            engine,
        )
        .expect("open ffi client");
        Self {
            client: Some(client),
            store,
            server: Some(server),
            server_rt: Some(server_rt),
            call_rt,
            profile,
        }
    }

    pub fn client(&self) -> &Arc<FfiClient> {
        self.client.as_ref().expect("client open")
    }

    pub fn server(&self) -> &TestServer {
        self.server.as_ref().expect("server running")
    }

    /// Runs an async test future on `call_rt` with a 20s deadline.
    pub fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.call_rt
            .block_on(async { tokio::time::timeout(Duration::from_secs(20), fut).await })
            .expect("ffi test future timed out")
    }

    /// Executes work on `server_rt`.
    pub fn on_server<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.server_rt.as_ref().expect("server_rt").block_on(fut)
    }

    /// Registers `username` + logs in against `server_id` and waits until connection reaches `Online`.
    pub fn register_and_login_online(&self, server_id: &str, username: &str) -> FfiAccountId {
        let client = self.client();
        let auth = client
            .server_auth(server_id.to_owned())
            .expect("server auth");
        let pass = "Aabcdefghijklmnop!".to_owned();
        let account = self.block_on(async {
            auth.register(username.to_owned(), pass.clone())
                .await
                .expect("register");
            auth.login(username.to_owned(), pass, 1000)
                .await
                .expect("login")
        });
        wait_account_online(client, account.clone());
        account
    }

    /// Shuts down `FfiClient` off-Tokio and stops the loopback `TestServer`.
    pub fn shutdown(mut self, timeout_ms: u64) -> FfiShutdownReport {
        let client = self.client.take().expect("client open");
        let report = client.shutdown(timeout_ms).expect("ffi shutdown");
        drop(client);
        if let (Some(server), Some(rt)) = (self.server.take(), self.server_rt.take()) {
            rt.block_on(server.shutdown()).expect("server shutdown");
            rt.shutdown_timeout(Duration::from_secs(2));
        }
        report
    }
}

/// Builds a loopback-enabled `FfiClientConfig` for integration tests.
pub fn client_config(path: &str) -> FfiClientConfig {
    FfiClientConfig {
        data_root_path: path.to_owned(),
        worker_threads: 2,
        credential_timeout_ms: 2_000,
        request_timeout_ms: 5_000,
        disk_budget_bytes: 64 * 1024 * 1024,
        call_join_timeout_ms: 2_000,
        call_drain_timeout_ms: 1_000,
        giphy_api_key: None,
        insecure_loopback: true,
    }
}

/// Waits on an OS barrier until `subscribe_connection` observes `FfiConnectionState::Online`.
pub fn wait_account_online(client: &Arc<FfiClient>, account: FfiAccountId) {
    struct OnlineListener(std::sync::mpsc::Sender<()>);
    impl FfiConnectionListener for OnlineListener {
        fn on_snapshot(&self, status: FfiConnectionStatus) {
            if matches!(status.state, FfiConnectionState::Online) {
                let _ = self.0.send(());
            }
        }
        fn on_update(&self, status: FfiConnectionStatus) {
            if matches!(status.state, FfiConnectionState::Online) {
                let _ = self.0.send(());
            }
        }
        fn on_closed(&self) {}
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let sub = client
        .subscribe_connection(account, Arc::new(OnlineListener(tx)))
        .expect("subscribe connection");
    rx.recv_timeout(Duration::from_secs(10))
        .expect("account reached online");
    sub.dispose();
}
