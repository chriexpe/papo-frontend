//! Integrated storage ownership tests; fake hooks do not implement sessions or media.

use crate::removal::{RemovalFuture, RemovalHookError, RemovalHooks};
use crate::secret::{SecretStoreBoundary, test_store::ScriptedSecretStore};
use crate::{AccountId, DataRoot, ServerAddress, Storage, StorageConfig, StorageError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::runtime::Runtime;

#[derive(Default)]
pub(crate) struct Hooks {
    pub(crate) fail_media: AtomicBool,
    pub(crate) fail_stop: AtomicBool,
    pub(crate) stops: AtomicUsize,
    pub(crate) media: AtomicUsize,
}
impl RemovalHooks for Hooks {
    fn stop_session(&self, _: AccountId) -> RemovalFuture<'_> {
        Box::pin(async {
            self.stops.fetch_add(1, Ordering::SeqCst);
            if self.fail_stop.load(Ordering::SeqCst) {
                Err(RemovalHookError::Incomplete)
            } else {
                Ok(())
            }
        })
    }
    fn delete_media_refs(&self, _: AccountId) -> RemovalFuture<'_> {
        Box::pin(async {
            self.media.fetch_add(1, Ordering::SeqCst);
            if self.fail_media.load(Ordering::SeqCst) {
                Err(RemovalHookError::Incomplete)
            } else {
                Ok(())
            }
        })
    }
}

pub(crate) struct Fixture {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) runtime: Option<Runtime>,
    pub(crate) storage: Option<Storage>,
    pub(crate) secrets: Arc<ScriptedSecretStore>,
    pub(crate) hooks: Arc<Hooks>,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let mut fixture = Self {
            directory: tempfile::tempdir().unwrap(),
            runtime: None,
            storage: None,
            secrets: Arc::new(ScriptedSecretStore::default()),
            hooks: Arc::new(Hooks::default()),
        };
        fixture.reopen();
        fixture
    }
    pub(crate) fn root(&self) -> DataRoot {
        DataRoot::new(self.directory.path())
    }
    pub(crate) fn storage(&self) -> &Storage {
        self.storage.as_ref().unwrap()
    }
    pub(crate) fn runtime(&self) -> &Runtime {
        self.runtime.as_ref().unwrap()
    }
    pub(crate) fn reopen(&mut self) {
        self.close();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let boundary = Arc::new(SecretStoreBoundary::new(
            self.secrets.clone(),
            std::time::Duration::from_secs(1),
        ));
        self.storage = Some(
            Storage::open(
                self.root(),
                &runtime,
                boundary,
                self.hooks.clone(),
                StorageConfig::default(),
            )
            .unwrap(),
        );
        self.runtime = Some(runtime);
    }
    pub(crate) fn close(&mut self) {
        if let Some(mut storage) = self.storage.take() {
            let result = storage.shutdown(&mut self.runtime);
            if !std::thread::panicking() {
                let report = result.unwrap();
                assert!(report.reads_drained);
                assert_eq!(
                    report.state.unpersisted,
                    [] as [crate::storage_worker::UnpersistedWrite; 0]
                );
                assert_eq!(
                    report.shared.unpersisted,
                    [] as [crate::storage_worker::UnpersistedWrite; 0]
                );
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn address(index: u32) -> ServerAddress {
    ServerAddress::parse(&format!("https://example.invalid/{index}")).unwrap()
}
pub(crate) fn account(server: &ServerAddress, user: u128) -> AccountId {
    AccountId::new(server.id(), papo_api::Uuid::from_u128(user))
}

#[test]
fn open_refuses_a_second_owner_before_creating_a_session() {
    let fixture = Fixture::new();
    let boundary = Arc::new(SecretStoreBoundary::new(
        fixture.secrets.clone(),
        std::time::Duration::from_secs(1),
    ));
    assert!(matches!(
        Storage::open(
            fixture.root(),
            fixture.runtime(),
            boundary,
            fixture.hooks.clone(),
            StorageConfig::default()
        ),
        Err(StorageError::ProfileInUse)
    ));
}

#[test]
fn lifecycle_calls_inside_runtime_are_rejected_without_dropping_runtime() {
    let mut fixture = Fixture::new();
    let handle = fixture.runtime().handle().clone();
    let guard = handle.enter();
    let error = fixture
        .storage
        .as_mut()
        .unwrap()
        .shutdown(&mut fixture.runtime)
        .unwrap_err();
    assert!(matches!(error, StorageError::RuntimeContext));
    assert!(fixture.runtime.is_some());
    drop(guard);
    fixture.close();
}

#[test]
fn owner_refuses_newer_schema_without_modifying_it() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let connection = rusqlite::Connection::open(root.state()).unwrap();
    connection
        .pragma_update(None, "user_version", crate::state_schema::VERSION + 1)
        .unwrap();
    drop(connection);
    let before = std::fs::read(root.state()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let boundary = Arc::new(SecretStoreBoundary::new(
        Arc::new(ScriptedSecretStore::default()),
        std::time::Duration::from_secs(1),
    ));
    assert!(matches!(
        Storage::open(
            root.clone(),
            &runtime,
            boundary,
            Arc::new(Hooks::default()),
            StorageConfig::default()
        ),
        Err(StorageError::NewerSchema { found: 4 })
    ));
    assert_eq!(std::fs::read(root.state()).unwrap(), before);
}
