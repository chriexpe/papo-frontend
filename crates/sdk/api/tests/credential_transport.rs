//! Observes public REST cookie behavior on loopback only; no paused time or timing assertions.

use papo_api::{CredentialSnapshot, RestClient};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

fn value() -> String {
    (b'A'..=b'Z').map(char::from).collect()
}

fn imported(base: &str, cookies: Vec<String>) -> CredentialSnapshot {
    let cookies = Value::Array(cookies.into_iter().map(Value::String).collect());
    CredentialSnapshot::import(
        &serde_json::to_vec(&json!({ "version": 1, "base": base, "cookies": cookies })).unwrap(),
    )
    .unwrap()
}

fn cookies(client: &RestClient) -> Vec<String> {
    let snapshot = client.credential_snapshot().unwrap();
    let envelope: Value = serde_json::from_slice(snapshot.expose_bytes()).unwrap();
    serde_json::from_value(envelope["cookies"].clone()).unwrap()
}

async fn server(responses: Vec<Vec<String>>) -> (String, JoinHandle<Vec<Option<String>>>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut observed = Vec::new();
        for cookies in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 1024];
                let read = socket.read(&mut bytes).await.unwrap();
                assert!(read > 0 && request.len() < 64 * 1024);
                request.extend_from_slice(&bytes[..read]);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            observed.push(request.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("cookie")
                    .then(|| value.trim().to_owned())
            }));
            let mut response = String::from(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n",
            );
            for cookie in cookies {
                response.push_str("Set-Cookie: ");
                response.push_str(&cookie);
                response.push_str("\r\n");
            }
            response.push_str("\r\n{}");
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        observed
    });
    (address, task)
}

#[tokio::test]
async fn restore_never_reuses_a_stale_response_jar() {
    let token = format!("Auth={}", value());
    let deletion = "Auth=; Path=/; Max-Age=0".to_owned();
    let (address, server) = server(vec![
        vec![format!("{token}; Path=/")],
        vec![deletion.clone()],
        vec![deletion],
        vec![],
    ])
    .await;
    let original = RestClient::loopback(&address).unwrap();
    original.server().await.unwrap();
    let snapshot = original.credential_snapshot().unwrap();
    let restored = RestClient::loopback_from_snapshot(&address, &snapshot).unwrap();
    original.server().await.unwrap();
    assert!(cookies(&original).is_empty());
    assert_eq!(cookies(&restored).len(), 1);
    restored.server().await.unwrap();
    assert!(cookies(&restored).is_empty());
    let independent = RestClient::loopback_from_snapshot(&address, &snapshot).unwrap();
    independent.server().await.unwrap();
    let observed = server.await.unwrap();
    assert!(observed[0].is_none());
    assert!(
        observed[1..]
            .iter()
            .all(|cookie| cookie.as_deref() == Some(token.as_str()))
    );
}

#[tokio::test]
async fn temporary_server_auth_is_replaced_by_session_auth_and_deletion_persists() {
    let temporary = format!("Auth={}", value());
    let session = format!("Auth={}", value().to_lowercase());
    let (address, server) = server(vec![
        vec![format!("{temporary}; Path=/; Max-Age=1800")],
        vec![format!("{session}; Path=/; Max-Age=2592000")],
        vec!["Auth=; Path=/; Max-Age=0".into()],
    ])
    .await;
    let client = RestClient::loopback(&address).unwrap();
    client.server().await.unwrap();
    assert_eq!(cookies(&client).len(), 1);
    client.server().await.unwrap();
    let saved = cookies(&client);
    assert_eq!(saved.len(), 1);
    assert!(saved[0].starts_with(&session));
    client.server().await.unwrap();
    let snapshot = client.credential_snapshot().unwrap();
    assert!(cookies(&RestClient::loopback_from_snapshot(&address, &snapshot).unwrap()).is_empty());
    let observed = server.await.unwrap();
    assert!(
        observed[1]
            .as_deref()
            .is_some_and(|cookie| cookie == temporary)
    );
    assert!(
        observed[2]
            .as_deref()
            .is_some_and(|cookie| cookie == session)
    );
}

#[tokio::test]
async fn relative_max_age_is_exported_as_absolute_expiry_without_renewal() {
    let (address, server) =
        server(vec![vec![format!("Auth={}; Path=/; Max-Age=300", value())]]).await;
    let client = RestClient::loopback(&address).unwrap();
    client.server().await.unwrap();
    let first = client.credential_snapshot().unwrap();
    let restored = RestClient::loopback_from_snapshot(&address, &first).unwrap();
    let second = restored.credential_snapshot().unwrap();
    #[expect(
        clippy::manual_assert_eq,
        reason = "Credential bytes must never appear in assertion diagnostics"
    )]
    {
        assert!(first.expose_bytes() == second.expose_bytes());
    }
    let saved = cookies(&restored);
    assert!(!saved[0].contains("Max-Age"));
    assert!(saved[0].contains("Expires="));
    server.await.unwrap();
}

#[tokio::test]
async fn default_response_path_is_preserved_in_snapshot() {
    let token = format!("Auth={}", value());
    let (address, server) = server(vec![vec![token.clone()], vec![]]).await;
    let address = format!("{address}api/auth/");
    let client = RestClient::loopback(&address).unwrap();
    client.server().await.unwrap();
    let snapshot = client.credential_snapshot().unwrap();
    let restored = RestClient::loopback_from_snapshot(&address, &snapshot).unwrap();
    let saved = cookies(&restored);
    assert!(saved[0].contains("Path=/api/auth"));
    assert!(!saved[0].contains("Domain="));
    restored.whoami().await.unwrap();
    assert!(
        server.await.unwrap()[1]
            .as_deref()
            .is_some_and(|cookie| cookie == token)
    );
}

#[tokio::test]
async fn secure_path_and_host_only_metadata_survive_public_restore() {
    let token = format!("Auth={}", value());
    let (address, server) = server(vec![vec![], vec![], vec![]]).await;
    let snapshot = imported(
        &address,
        vec![format!("{token}; HttpOnly; Secure; Path=/auth")],
    );
    let strict = RestClient::from_snapshot(&address, &snapshot).unwrap();
    strict.whoami().await.unwrap();
    let relaxed = RestClient::loopback_from_snapshot(&address, &snapshot).unwrap();
    relaxed.whoami().await.unwrap();
    relaxed.server().await.unwrap();
    let saved = cookies(&relaxed);
    assert!(
        saved[0].contains("Secure")
            && saved[0].contains("HttpOnly")
            && !saved[0].contains("Domain=")
    );
    let observed = server.await.unwrap();
    assert!(observed[0].is_none());
    assert!(observed[1].as_deref().is_some_and(|cookie| cookie == token));
    assert!(observed[2].is_none());
}

#[tokio::test]
async fn more_specific_cookie_path_is_sent_first_after_restore() {
    let broad = format!("Auth={}", value());
    let specific = format!("Auth={}", value().to_lowercase());
    let (address, server) = server(vec![vec![]]).await;
    let snapshot = imported(
        &address,
        vec![
            format!("{broad}; Path=/"),
            format!("{specific}; Path=/auth"),
        ],
    );
    let restored = RestClient::loopback_from_snapshot(&address, &snapshot).unwrap();
    restored.whoami().await.unwrap();
    let observed = server.await.unwrap();
    assert!(
        observed[0]
            .as_deref()
            .is_some_and(|header| header.starts_with(&specific))
    );
}
