//! Owns real-loopback multi-user presence verification; leaves counted-state edge cases beside the registry.

use std::time::Duration;

use futures_util::StreamExt;
use papo_test_server::{Config, Error, TestServer};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::client::IntoClientRequest};

async fn sign_in(server: &TestServer, username: &str) -> Result<WebSocketStream<TcpStream>, Error> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| Error::Connection)?;
    let password: String = std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect();
    let body = json!({"username": username, "password": password});
    let registered = client
        .post(format!("{}/auth/register", server.base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(registered.status().as_u16(), 201);
    let signed_in = client
        .post(format!("{}/auth/login", server.base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(signed_in.status().as_u16(), 200);
    let session = signed_in
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .ok_or(Error::Connection)?;
    let mut request = server
        .gateway_url()
        .into_client_request()
        .map_err(|_| Error::Connection)?;
    request
        .headers_mut()
        .insert("Cookie", session.parse().map_err(|_| Error::Connection)?);
    let stream = TcpStream::connect(server.address())
        .await
        .map_err(|_| Error::Connection)?;
    let (socket, _) = tokio::time::timeout(Duration::from_secs(5), client_async(request, stream))
        .await
        .map_err(|_| Error::Connection)?
        .map_err(|_| Error::Connection)?;
    Ok(socket)
}

async fn event(socket: &mut WebSocketStream<TcpStream>) -> Result<Value, Error> {
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| Error::Connection)?
        .ok_or(Error::Connection)?
        .map_err(|_| Error::Connection)?;
    serde_json::from_slice(&message.into_data()).map_err(|_| Error::Connection)
}

#[tokio::test]
async fn new_user_receives_full_snapshot_and_existing_user_receives_transitions()
-> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let mut first = sign_in(&server, "first").await?;
    assert_eq!(
        event(&mut first).await?["members"].as_array().map(Vec::len),
        Some(1)
    );
    let mut second = sign_in(&server, "second").await?;
    assert_eq!(
        event(&mut second).await?["members"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(event(&mut first).await?["type"], "user_join");
    let online = event(&mut first).await?;
    assert_eq!(
        (online["type"].as_str(), online["status"].as_str()),
        (Some("presence_update"), Some("online"))
    );
    second.close(None).await.map_err(|_| Error::Connection)?;
    let offline = event(&mut first).await?;
    assert_eq!(
        (offline["type"].as_str(), offline["status"].as_str()),
        (Some("presence_update"), Some("offline"))
    );
    first.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}
