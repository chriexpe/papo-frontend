//! Verifies restored production transports against loopback sockets; deadline behavior is tested in memory.

use papo_api::{
    CredentialSnapshot, RestClient,
    gateway::{Event, Gateway, KnownEvent},
    wire::{LoginRequest, RegisterRequest},
};
use papo_test_server::{Config, TestServer};

#[tokio::test]
async fn restored_independent_transport_authenticates_rest_and_single_gateway_socket() {
    let server = TestServer::start(Config::default()).await.unwrap();
    let address = server.base_url();
    let original = RestClient::loopback(&address).unwrap();
    let password: String = (b'A'..=b'Z').map(char::from).chain(['!']).collect();
    original
        .register(&RegisterRequest {
            username: "tester".into(),
            password: password.clone(),
        })
        .await
        .unwrap();
    original
        .login(&LoginRequest {
            username: "tester".into(),
            password,
        })
        .await
        .unwrap();
    let snapshot = original.credential_snapshot().unwrap();
    let imported = CredentialSnapshot::import(snapshot.expose_bytes()).unwrap();
    let restored = RestClient::loopback_from_snapshot(&address, &imported).unwrap();
    drop(original);
    assert!(restored.whoami().await.unwrap().id.is_some());
    let mut gateway = Gateway::connect(&restored).await.unwrap();
    assert!(matches!(
        gateway.next().await.unwrap(),
        Some(Event::Known(KnownEvent::PresenceSync(_)))
    ));
    gateway.close().await.unwrap();
    restored.logout().await.unwrap();
    let cleared = restored.credential_snapshot().unwrap();
    let empty = RestClient::loopback_from_snapshot(&address, &cleared).unwrap();
    assert!(matches!(
        Gateway::connect(&empty).await,
        Err(papo_api::gateway::Error::Upgrade(401))
    ));
    server.shutdown().await.unwrap();
}
