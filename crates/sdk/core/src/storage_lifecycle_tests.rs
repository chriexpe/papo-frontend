//! External lifecycle regression tests; channels control blocking work without sleeps.

use crate::secret::{SecretStoreBoundary, test_store::ScriptedSecretStore};
use crate::storage_tests::Hooks;
use crate::{DataRoot, Storage, StorageConfig, StorageError};
use std::{sync::Arc, time::Duration};

#[test]
fn missed_read_deadline_retains_profile_lock_until_blocking_query_really_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let boundary = Arc::new(SecretStoreBoundary::new(
        Arc::new(ScriptedSecretStore::default()),
        Duration::from_secs(1),
    ));
    let config = StorageConfig {
        drain_timeout: Duration::ZERO,
        runtime_stop_timeout: Duration::ZERO,
        ..StorageConfig::default()
    };
    let mut storage = Storage::open(
        root.clone(),
        &runtime,
        boundary,
        Arc::new(Hooks::default()),
        config,
    )
    .unwrap();
    let readers = storage.readers();
    let path = root.state();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let query_thread = std::thread::spawn(move || {
        let query_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        query_runtime
            .block_on(readers.query(path, move |tx| {
                let _: i64 = tx.query_row("SELECT count(*) FROM servers", [], |r| r.get(0))?;
                entered_tx.send(()).unwrap();
                let _ = release_rx.recv();
                Ok(())
            }))
            .unwrap();
    });
    entered_rx.recv().unwrap();
    let mut runtime = Some(runtime);
    let report = storage.shutdown(&mut runtime).unwrap();
    assert!(!report.reads_drained);
    drop(storage);
    let refused = matches!(root.acquire(), Err(StorageError::ProfileInUse));
    release_tx.send(()).unwrap();
    query_thread.join().unwrap();
    assert!(refused);
    assert!(root.acquire().is_ok());
}

#[test]
fn open_inside_tokio_is_rejected_before_taking_the_profile_lock() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let boundary = Arc::new(SecretStoreBoundary::new(
        Arc::new(ScriptedSecretStore::default()),
        Duration::from_secs(1),
    ));
    runtime.block_on(async {
        assert!(matches!(
            Storage::open(
                root.clone(),
                &runtime,
                boundary,
                Arc::new(Hooks::default()),
                StorageConfig::default()
            ),
            Err(StorageError::RuntimeContext)
        ));
    });
    assert!(root.acquire().is_ok());
}
