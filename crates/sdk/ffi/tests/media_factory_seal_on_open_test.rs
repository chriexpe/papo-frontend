//! Verifies `FfiClient::open` seals the global media engine factory slot in an isolated process
//! binary so any post-open registration is rejected with `ClientAlreadyOpened`.

use std::sync::Arc;

use papo_ffi::{
    FfiClient, FfiClientConfig, FfiSecretKey, FfiSecretRead, FfiSecretStore, FfiSecretStoreError,
    MediaFactoryRegistrationError, register_media_engine_factory_once,
};
use papo_sdk::FakeMediaFactory;

struct EmptyStore;
impl FfiSecretStore for EmptyStore {
    fn read(&self, _key: FfiSecretKey) -> Result<FfiSecretRead, FfiSecretStoreError> {
        Ok(FfiSecretRead {
            value: None,
            settled: true,
        })
    }
    fn write(&self, _key: FfiSecretKey, _value: Vec<u8>) -> Result<(), FfiSecretStoreError> {
        Ok(())
    }
    fn delete(&self, _key: FfiSecretKey) -> Result<(), FfiSecretStoreError> {
        Ok(())
    }
}

#[test]
fn open_seals_media_factory_slot_against_post_open_registration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = FfiClientConfig {
        data_root_path: dir.path().to_str().unwrap().to_owned(),
        worker_threads: 1,
        credential_timeout_ms: 1_000,
        request_timeout_ms: 3_000,
        disk_budget_bytes: 16 * 1024 * 1024,
        call_join_timeout_ms: 1_000,
        call_drain_timeout_ms: 500,
        giphy_api_key: None,
        insecure_loopback: true,
    };
    let client = FfiClient::open(cfg, Arc::new(EmptyStore)).expect("open seals slot");

    let late_engine: Arc<dyn papo_sdk::MediaEngineFactory> = Arc::new(FakeMediaFactory::new());
    assert_eq!(
        register_media_engine_factory_once(late_engine),
        Err(MediaFactoryRegistrationError::ClientAlreadyOpened)
    );

    let report = client.shutdown(500).expect("clean shutdown");
    assert!(report.reads_drained);
}
