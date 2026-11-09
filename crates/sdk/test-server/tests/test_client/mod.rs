//! Owns bounded TCP account/socket test setup; leaves resource actions and assertions in integration tests.

use std::time::Duration;

use futures_util::StreamExt;
use papo_api::gateway::Event;
use papo_test_server::{Error, TestServer};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::client::IntoClientRequest};

pub(crate) struct Account {
    pub(crate) http: reqwest::Client,
    pub(crate) id: String,
    pub(crate) cookie: String,
    pub(crate) base: String,
}

impl Account {
    pub(crate) async fn register(server: &TestServer, name: &str) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| Error::Connection)?;
        let password: String = std::iter::once('A')
            .chain((0_u8..16).map(|index| char::from(b'a' + index)))
            .chain(std::iter::once('!'))
            .collect();
        let body = json!({"username": name, "password": password});
        let response = http
            .post(format!("{}/auth/register", server.base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        assert_eq!(response.status().as_u16(), 201);
        let user: Value = response.json().await.map_err(|_| Error::Connection)?;
        let response = http
            .post(format!("{}/auth/login", server.base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        let cookie = response
            .headers()
            .get("set-cookie")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .ok_or(Error::Connection)?
            .to_owned();
        Ok(Self {
            http,
            id: user["id"].as_str().ok_or(Error::Connection)?.to_owned(),
            cookie,
            base: server.base_url(),
        })
    }

    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.http
            .get(format!("{}{path}", self.base))
            .header("Cookie", &self.cookie)
            .send()
            .await
            .map_err(|_| Error::Connection)?
            .error_for_status()
            .map_err(|_| Error::Connection)?
            .json()
            .await
            .map_err(|_| Error::Connection)
    }

    pub(crate) async fn socket(
        &self,
        server: &TestServer,
    ) -> Result<WebSocketStream<TcpStream>, Error> {
        let mut request = server
            .gateway_url()
            .into_client_request()
            .map_err(|_| Error::Connection)?;
        request.headers_mut().insert(
            "Cookie",
            self.cookie.parse().map_err(|_| Error::Connection)?,
        );
        let stream = TcpStream::connect(server.address())
            .await
            .map_err(|_| Error::Connection)?;
        let (socket, _) =
            tokio::time::timeout(Duration::from_secs(5), client_async(request, stream))
                .await
                .map_err(|_| Error::Connection)?
                .map_err(|_| Error::Connection)?;
        Ok(socket)
    }
}

pub(crate) async fn next(socket: &mut WebSocketStream<TcpStream>) -> Result<Event, Error> {
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| Error::Connection)?
        .ok_or(Error::Connection)?
        .map_err(|_| Error::Connection)?;
    Event::decode(&message.into_data()).map_err(|_| Error::Connection)
}

pub(crate) async fn resource_event(
    socket: &mut WebSocketStream<TcpStream>,
) -> Result<Event, Error> {
    loop {
        let event = next(socket).await?;
        if !matches!(event.kind(), "presence_sync" | "presence_update") {
            return Ok(event);
        }
    }
}
