//! Exercises production transports against the deterministic loopback fixture; no external hosts.

use papo_api::{
    Error, ProblemKind, RestClient,
    gateway::{CloseReason, Command, Event, Gateway, KnownEvent},
    wire::{LoginRequest, RegisterRequest},
};
use papo_test_server::{Config, Failure, GatewayStep, RestStep, TestServer};
use serde_json::json;

async fn signed_in(config: Config) -> (TestServer, RestClient) {
    let server = TestServer::start(config).await.unwrap();
    let client = RestClient::loopback(&server.base_url()).unwrap();
    let password: String = (0_u8..16)
        .map(|i| char::from(b'A' + i))
        .chain(['!'])
        .collect();
    client
        .register(&RegisterRequest {
            username: "tester".into(),
            password: password.clone(),
        })
        .await
        .unwrap();
    client
        .login(&LoginRequest {
            username: "tester".into(),
            password,
        })
        .await
        .unwrap();
    (server, client)
}

#[tokio::test]
async fn cookies_authenticate_rest_and_gateway_and_logout_deletes_cookie() {
    let (server, client) = signed_in(Config::default()).await;
    assert!(client.whoami().await.unwrap().id.is_some());
    let mut gateway = Gateway::connect(&client).await.unwrap();
    assert!(matches!(
        gateway.next().await.unwrap(),
        Some(Event::Known(KnownEvent::PresenceSync(_)))
    ));
    gateway.close().await.unwrap();
    client.logout().await.unwrap();
    assert!(matches!(
        client.whoami().await,
        Err(Error::Problem { status: 401, .. })
    ));
    assert!(matches!(
        Gateway::connect(&client).await,
        Err(papo_api::gateway::Error::Upgrade(401))
    ));
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn backend_problem_keeps_status_and_classifies_without_formatting_payload() {
    let server = TestServer::start(Config {
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 429,
                slug: "future-problem".into(),
            }),
        }],
        ..Config::default()
    })
    .await
    .unwrap();
    let client = RestClient::loopback(&server.base_url()).unwrap();
    let error = client.server().await.err().unwrap();
    assert_eq!(format!("{error:?}"), "backend rejected request (429)");
    match error {
        Error::Problem { status, problem } => {
            assert_eq!(status, 429);
            assert_eq!(problem.kind(), ProblemKind::Unknown);
        }
        _ => panic!("expected a problem"),
    }
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn future_events_duplicates_and_normal_close_preserve_wire_order() {
    let config = Config {
        gateway: vec![
            GatewayStep::Event {
                event: json!({"type":"future_event", "id":1}),
            },
            GatewayStep::Event {
                event: json!({"type":"heartbeat_ack"}),
            },
            GatewayStep::Event {
                event: json!({"type":"heartbeat_ack"}),
            },
            GatewayStep::Disconnect,
        ],
        ..Config::default()
    };
    let (server, client) = signed_in(config).await;
    let mut gateway = Gateway::connect(&client).await.unwrap();
    gateway.next().await.unwrap();
    let Some(Event::Unknown(event)) = gateway.next().await.unwrap() else {
        panic!("expected unknown");
    };
    assert_eq!(event.kind, "future_event");
    assert_eq!(event.payload["id"], 1);
    for _ in 0..2 {
        assert_eq!(
            gateway.next().await.unwrap().unwrap().kind(),
            "heartbeat_ack"
        );
    }
    assert!(matches!(
        gateway.next().await.unwrap(),
        Some(Event::Closed(CloseReason::NoStatus))
    ));
    assert!(gateway.next().await.unwrap().is_none());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_known_event_is_an_error_instead_of_unknown() {
    let (server, client) = signed_in(Config {
        gateway: vec![GatewayStep::Event {
            event: json!({"type":"message_delete", "id":42}),
        }],
        ..Config::default()
    })
    .await;
    let mut gateway = Gateway::connect(&client).await.unwrap();
    gateway.next().await.unwrap();
    assert!(matches!(
        gateway.next().await,
        Err(papo_api::gateway::Error::Decode)
    ));
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_heartbeat_is_acknowledged_without_application_activity() {
    let (server, client) = signed_in(Config::default()).await;
    let mut gateway = Gateway::connect(&client).await.unwrap();
    gateway.next().await.unwrap();
    gateway.send(&Command::Heartbeat).await.unwrap();
    assert_eq!(
        gateway.next().await.unwrap().unwrap().kind(),
        "heartbeat_ack"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn protocol_pings_binary_events_and_typed_close_are_serviced() {
    let (server, client) = signed_in(Config {
        gateway: vec![
            GatewayStep::Ping,
            GatewayStep::Raw {
                data: br#"{"type":"heartbeat_ack"}"#.to_vec(),
            },
            GatewayStep::Close { code: 1008 },
        ],
        ..Config::default()
    })
    .await;
    let mut gateway = Gateway::connect(&client).await.unwrap();
    gateway.next().await.unwrap();
    assert_eq!(
        gateway.next().await.unwrap().unwrap().kind(),
        "heartbeat_ack"
    );
    assert!(matches!(
        gateway.next().await.unwrap(),
        Some(Event::Closed(CloseReason::Policy))
    ));
    server.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn missing_heartbeat_ack_times_out_without_reconnecting() {
    let (server, client) = signed_in(Config {
        ignore_heartbeats: true,
        ..Config::default()
    })
    .await;
    let mut gateway = Gateway::connect(&client).await.unwrap();
    gateway.next().await.unwrap();
    assert!(matches!(
        gateway.next().await,
        Err(papo_api::gateway::Error::HeartbeatTimeout)
    ));
    assert!(gateway.next().await.unwrap().is_none());
    server.shutdown().await.unwrap();
}
