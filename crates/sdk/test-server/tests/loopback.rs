//! Owns actual loopback REST/gateway integration tests; leaves pure handler behavior beside implementation.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use papo_test_server::{Config, Error, Failure, GatewayStep, RestStep, TestServer};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{Message, client::IntoClientRequest},
};

fn credentials() -> Value {
    let password: String = std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect();
    json!({"username": "tester", "password": password})
}

fn client() -> Result<Client, Error> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| Error::Connection)
}

async fn post(
    client: &Client,
    server: &TestServer,
    path: &str,
    body: &Value,
    cookie: Option<&str>,
) -> Result<Response, Error> {
    let mut request = client
        .post(format!("{}{path}", server.base_url()))
        .json(body);
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    request.send().await.map_err(|_| Error::Connection)
}

fn cookie(response: &Response) -> Result<String, Error> {
    response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_owned)
        .ok_or(Error::Connection)
}

async fn signed_in(client: &Client, server: &TestServer) -> Result<String, Error> {
    let registration = post(client, server, "/auth/register", &credentials(), None).await?;
    assert_eq!(registration.status().as_u16(), 201);
    let login = post(client, server, "/auth/login", &credentials(), None).await?;
    assert_eq!(login.status().as_u16(), 200);
    cookie(&login)
}

async fn connect(
    server: &TestServer,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<WebSocketStream<TcpStream>, u16> {
    let mut request = server
        .gateway_url()
        .into_client_request()
        .map_err(|_| 0_u16)?;
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert("Cookie", cookie.parse().map_err(|_| 0_u16)?);
    }
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", origin.parse().map_err(|_| 0_u16)?);
    }
    let stream = TcpStream::connect(server.address())
        .await
        .map_err(|_| 0_u16)?;
    let result = tokio::time::timeout(Duration::from_secs(5), client_async(request, stream))
        .await
        .map_err(|_| 0_u16)?;
    result
        .map(|(socket, _)| socket)
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
            _ => 0,
        })
}

async fn event(socket: &mut WebSocketStream<TcpStream>) -> Result<Value, Error> {
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| Error::Connection)?
        .ok_or(Error::Connection)?
        .map_err(|_| Error::Connection)?;
    let Message::Text(text) = message else {
        return Err(Error::Connection);
    };
    serde_json::from_str(&text).map_err(|_| Error::Connection)
}

#[tokio::test]
async fn auth_cookie_lifecycle_matches_backend_without_exposing_token_in_body() -> Result<(), Error>
{
    let server = TestServer::start(Config::default()).await?;
    let client = client()?;
    let registration = post(&client, &server, "/auth/register", &credentials(), None).await?;
    assert_eq!(registration.status().as_u16(), 201);
    assert!(!registration.headers().contains_key("set-cookie"));
    let login = post(&client, &server, "/auth/login", &credentials(), None).await?;
    let session = cookie(&login)?;
    let attributes = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .ok_or(Error::Connection)?;
    assert!(
        attributes.contains("HttpOnly")
            && attributes.contains("Secure")
            && attributes.contains("SameSite=Strict")
    );
    let body: Value = login.json().await.map_err(|_| Error::Connection)?;
    assert!(body.get("token").is_none());
    assert_eq!(body["connection_violation"], false);
    let identity = client
        .get(format!("{}/auth/whoami", server.base_url()))
        .header("Cookie", &session)
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(identity.status().as_u16(), 200);
    let logout = post(&client, &server, "/auth/logout", &json!({}), Some(&session)).await?;
    assert_eq!(logout.status().as_u16(), 204);
    let identity = client
        .get(format!("{}/auth/whoami", server.base_url()))
        .header("Cookie", &session)
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(identity.status().as_u16(), 401);
    server.shutdown().await
}

#[tokio::test]
async fn invalid_auth_and_malformed_body_use_problem_json() -> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let client = client()?;
    let login = post(&client, &server, "/auth/login", &credentials(), None).await?;
    assert_eq!(login.status().as_u16(), 401);
    assert_eq!(
        login
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    let body: Value = login.json().await.map_err(|_| Error::Connection)?;
    assert!(body["type"].as_str().is_some_and(|kind| {
        kind.strip_prefix(&server.base_url()) == Some("/errors/invalid-credentials")
    }));
    let malformed = client
        .post(format!("{}/auth/login", server.base_url()))
        .header("Content-Type", "application/json")
        .body("{")
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(malformed.status().as_u16(), 400);
    server.shutdown().await
}

#[tokio::test]
async fn private_server_temporary_cookie_allows_registration_but_not_gateway() -> Result<(), Error>
{
    let password = credentials()["password"]
        .as_str()
        .ok_or(Error::Configuration)?
        .to_owned();
    let config = Config {
        public: false,
        server_password: Some(password.clone()),
        ..Config::default()
    };
    let server = TestServer::start(config).await?;
    let client = client()?;
    assert_eq!(
        post(&client, &server, "/auth/register", &credentials(), None)
            .await?
            .status()
            .as_u16(),
        401
    );
    let rejected = post(
        &client,
        &server,
        "/auth/login_server",
        &json!({"server_password": password.to_lowercase()}),
        None,
    )
    .await?;
    assert_eq!(rejected.status().as_u16(), 401);
    let login = post(
        &client,
        &server,
        "/auth/login_server",
        &json!({"server_password": password}),
        None,
    )
    .await?;
    assert_eq!(login.status().as_u16(), 204);
    let temporary = cookie(&login)?;
    assert_eq!(
        post(
            &client,
            &server,
            "/auth/register",
            &credentials(),
            Some(&temporary)
        )
        .await?
        .status()
        .as_u16(),
        201
    );
    assert_eq!(
        connect(&server, Some(&temporary), None).await.err(),
        Some(401)
    );
    server.shutdown().await
}

#[tokio::test]
async fn discovery_failure_is_consumed_once_then_public_discovery_recovers() -> Result<(), Error> {
    let config = Config {
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 503,
                slug: "internal".into(),
            }),
        }],
        ..Config::default()
    };
    let server = TestServer::start(config).await?;
    let client = client()?;
    let first = client
        .get(format!("{}/server", server.base_url()))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(first.status().as_u16(), 503);
    let second = client
        .get(format!("{}/server", server.base_url()))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(second.status().as_u16(), 200);
    server.shutdown().await
}

#[tokio::test]
async fn gateway_requires_cookie_and_checks_browser_origin_before_upgrade() -> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    assert_eq!(connect(&server, None, None).await.err(), Some(401));
    let session = signed_in(&client()?, &server).await?;
    assert_eq!(
        connect(&server, Some(&session), Some(&server.base_url()))
            .await
            .err(),
        Some(403)
    );
    server.shutdown().await
}

#[tokio::test]
async fn gateway_sends_presence_acks_heartbeats_and_rejects_invalid_events() -> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let session = signed_in(&client()?, &server).await?;
    let mut socket = connect(&server, Some(&session), None)
        .await
        .map_err(|_| Error::Connection)?;
    let presence = event(&mut socket).await?;
    assert_eq!(presence["type"], "presence_sync");
    assert_eq!(presence["members"].as_array().map(Vec::len), Some(1));
    socket
        .send(Message::text(json!({"type": "heartbeat"}).to_string()))
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(event(&mut socket).await?, json!({"type": "heartbeat_ack"}));
    socket
        .send(Message::text("{"))
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(event(&mut socket).await?["type"], "error");
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn gateway_replays_ordered_duplicates_then_disconnects() -> Result<(), Error> {
    let first = json!({"type": "role_remove", "user_id": 2, "role_id": 1});
    let second = json!({"type": "role_add", "user_id": 2, "role_id": 1});
    let config = Config {
        gateway: vec![
            GatewayStep::Delay { delay_ms: 1 },
            GatewayStep::Event {
                event: first.clone(),
            },
            GatewayStep::Event {
                event: second.clone(),
            },
            GatewayStep::Event {
                event: first.clone(),
            },
            GatewayStep::Disconnect,
        ],
        ..Config::default()
    };
    let server = TestServer::start(config).await?;
    let session = signed_in(&client()?, &server).await?;
    let mut socket = connect(&server, Some(&session), None)
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(event(&mut socket).await?["type"], "presence_sync");
    assert_eq!(event(&mut socket).await?, first);
    assert_eq!(event(&mut socket).await?, second);
    assert_eq!(event(&mut socket).await?, first);
    let close = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| Error::Connection)?
        .ok_or(Error::Connection)?
        .map_err(|_| Error::Connection)?;
    assert!(matches!(close, Message::Close(_)));
    server.shutdown().await
}
