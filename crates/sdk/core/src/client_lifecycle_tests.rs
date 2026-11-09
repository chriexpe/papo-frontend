//! Client ownership and exit fences without networking; storage's own tests cover drain deadlines.

use crate::{
    Client, ClientConfig, ClientError, DataRoot,
    secret::test_store::{Operation, ScriptedSecretStore},
};
use std::sync::Arc;

#[test]
fn profile_in_use_is_surfaced_before_any_credential_call() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let first = Arc::new(ScriptedSecretStore::default());
    let second = Arc::new(ScriptedSecretStore::default());
    let mut client = Client::open(root.clone(), first, ClientConfig::default()).unwrap();
    assert!(matches!(
        Client::open(root, second.clone(), ClientConfig::default()),
        Err(ClientError::ProfileInUse)
    ));
    assert_eq!(second.calls(Operation::Read), 0);
    client.shutdown().unwrap();
}
#[test]
fn shutdown_closes_surviving_registry_stream_and_service_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let handle = client.handle();
    let mut registry = handle.subscribe_registry();
    registry.borrow_and_update();
    let report = client.shutdown().unwrap();
    assert!(report.reads_drained && report.state.finished && report.shared.finished);
    assert!(registry.has_changed().is_err());
    assert!(handle.is_closed());
}
#[test]
fn lifecycle_calls_inside_tokio_are_rejected_without_consuming_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let executor = client.handle.inner.runtime.clone();
    executor.block_on(async {
        assert!(matches!(
            client.shutdown(),
            Err(ClientError::RuntimeContext)
        ));
        assert!(matches!(
            Client::open(
                DataRoot::new(directory.path().join("other")),
                Arc::new(ScriptedSecretStore::default()),
                ClientConfig::default()
            ),
            Err(ClientError::RuntimeContext)
        ));
    });
    assert!(!client.handle().is_closed());
    client.shutdown().unwrap();
}
#[test]
fn dropping_client_from_a_tracked_task_does_not_wait_for_itself() {
    let directory = tempfile::tempdir().unwrap();
    let client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            worker_threads: 1,
            ..ClientConfig::default()
        },
    )
    .unwrap();
    let executor = client.handle.inner.runtime.clone();
    let token = client.handle.inner.tasks.token();
    let handle = client.handle();
    let registry = handle.subscribe_registry();
    let (finished, receive) = std::sync::mpsc::channel();
    executor.spawn(async move {
        let _token = token;
        drop(client);
        finished.send(()).unwrap();
    });
    receive.recv().unwrap();
    assert!(handle.is_closed());
    assert!(registry.has_changed().is_err());
}

#[test]
fn server_removal_closes_bootstrap_auth_even_after_same_server_is_added_again() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let handle = client.handle();
    let address = crate::ServerAddress::parse("https://example.invalid").unwrap();
    let id = address.id();
    client
        .block_on(async {
            handle
                .inner
                .registry
                .add_server(address.clone(), crate::UnixMicros(0))
                .await
                .unwrap();
            handle.inner.publish_registry().await.unwrap();
        })
        .unwrap();
    let old = handle.auth(id).unwrap();
    let mut states = old.subscribe();
    states.borrow_and_update();
    client.block_on(handle.remove_server(id)).unwrap().unwrap();
    states.borrow_and_update();
    assert!(states.has_changed().is_err());
    client
        .block_on(async {
            handle
                .inner
                .registry
                .add_server(address, crate::UnixMicros(0))
                .await
                .unwrap();
            handle.inner.publish_registry().await.unwrap();
            assert!(matches!(
                old.server_login(papo_api::wire::LoginServerRequest {
                    server_password: String::new()
                })
                .await,
                Err(ClientError::Closed)
            ));
        })
        .unwrap();
    assert!(handle.auth(id).is_ok());
    client.shutdown().unwrap();
}
