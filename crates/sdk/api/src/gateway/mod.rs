//! Owns a single gateway connection, heartbeat and decoding; leaves reconnect and generations to the SDK.

mod known;
use futures_util::{SinkExt, StreamExt};
pub use known::*;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};

/// Go serializes nil slices as null; consumers receive an empty replacement list.
fn nullable_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<Vec<T>> as serde::Deserialize>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// A future event retains its exact JSON without being confused with malformed known events.
#[derive(Clone)]
pub struct UnknownEvent {
    /// Server-provided discriminator, deliberately absent from Debug output.
    pub kind: String,
    /// Original JSON object, for a future protocol consumer.
    pub payload: serde_json::Value,
}

/// Decoded live traffic or a typed termination.
#[derive(Clone)]
#[non_exhaustive]
pub enum Event {
    /// A verified backend event.
    Known(KnownEvent),
    /// A well-formed event with a future discriminator.
    Unknown(UnknownEvent),
    /// The peer ended the connection. Normal closure does not imply logout.
    Closed(CloseReason),
}

/// WebSocket close meaning. The backend currently uses normal closure even for revocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseReason {
    /// RFC 6455 normal closure (1000).
    Normal,
    /// The peer is going away (1001).
    GoingAway,
    /// Protocol failure (1002).
    Protocol,
    /// Policy rejection (1008), without inventing an authentication meaning.
    Policy,
    /// Backend failure (1011).
    ServerError,
    /// A close frame without status.
    NoStatus,
    /// The connection ended without a close frame.
    Abnormal,
    /// An unrecognized status code; server text is discarded.
    Other(u16),
}

impl From<u16> for CloseReason {
    fn from(code: u16) -> Self {
        match code {
            1000 => Self::Normal,
            1001 => Self::GoingAway,
            1002 => Self::Protocol,
            1008 => Self::Policy,
            1011 => Self::ServerError,
            1005 => Self::NoStatus,
            _ => Self::Other(code),
        }
    }
}

/// Sanitized gateway failures contain no URL, credential or server payload.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The connection could not be established or used.
    #[error("gateway transport failed")]
    Transport,
    /// A rejected upgrade retains only its status.
    #[error("gateway upgrade rejected ({0})")]
    Upgrade(u16),
    /// Malformed JSON or a malformed known event.
    #[error("invalid gateway event")]
    Decode,
    /// An application heartbeat was not acknowledged before the next deadline.
    #[error("gateway heartbeat timed out")]
    HeartbeatTimeout,
}

impl Event {
    /// Decodes JSON without panicking; unknown discriminators preserve their payload.
    /// # Errors
    /// Rejects malformed JSON, missing/stringless discriminators and invalid known payloads.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let payload: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| Error::Decode)?;
        let kind = payload
            .get("type")
            .and_then(serde_json::Value::as_str)
            .ok_or(Error::Decode)?;
        if KnownEvent::recognizes(kind) {
            serde_json::from_value(payload)
                .map(Self::Known)
                .map_err(|_| Error::Decode)
        } else {
            Ok(Self::Unknown(UnknownEvent {
                kind: kind.into(),
                payload,
            }))
        }
    }

    /// Returns a static diagnostic kind. Unknown discriminator text is never returned.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Known(event) => event.kind(),
            Self::Unknown(_) => "unknown",
            Self::Closed(_) => "closed",
        }
    }

    /// Returns only valid UUID values from top-level identity fields, never arbitrary strings.
    #[must_use]
    pub fn ids(&self) -> Vec<crate::Uuid> {
        let Self::Known(event) = self else {
            return Vec::new();
        };
        let Ok(value) = serde_json::to_value(event) else {
            return Vec::new();
        };
        [
            "id",
            "channel_id",
            "message_id",
            "user_id",
            "author_id",
            "role_id",
            "attachment_id",
        ]
        .iter()
        .filter_map(|key| value.get(key)?.as_str()?.parse().ok())
        .collect()
    }
}

/// A single socket; callers must poll `next` continuously to service heartbeats.
/// Dropping the socket cancels it. No background task, retry or reconnect is created.
pub struct Gateway {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    heartbeat: tokio::time::Interval,
    awaiting_ack: bool,
    ended: bool,
}

impl Gateway {
    /// Connects to the REST client's server, with its in-memory auth cookie.
    /// # Errors
    /// Returns a sanitized upgrade or transport failure.
    pub async fn connect(client: &crate::RestClient) -> Result<Self, Error> {
        let mut endpoint = client.endpoint("ws").map_err(|_| Error::Transport)?;
        let scheme = if endpoint.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        endpoint.set_scheme(scheme).map_err(|()| Error::Transport)?;
        let mut request = endpoint
            .as_str()
            .into_client_request()
            .map_err(|_| Error::Transport)?;
        if let Some(cookie) = client.gateway_cookie() {
            request.headers_mut().insert("Cookie", cookie);
        }
        let config = WebSocketConfig::default().max_message_size(Some(8 * 1024 * 1024));
        let (socket, _) =
            tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                .await
                .map_err(|err| {
                    if let tokio_tungstenite::tungstenite::Error::Http(response) = err {
                        Error::Upgrade(response.status().as_u16())
                    } else {
                        Error::Transport
                    }
                })?;
        let heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(25),
            Duration::from_secs(25),
        );
        Ok(Self {
            socket,
            heartbeat,
            awaiting_ack: false,
            ended: false,
        })
    }

    /// Reads the next event while sending application heartbeats and responding to protocol pings.
    /// # Errors
    /// Returns malformed-event, timeout or transport failures. Recovery belongs to the SDK.
    pub async fn next(&mut self) -> Result<Option<Event>, Error> {
        if self.ended {
            return Ok(None);
        }
        loop {
            tokio::select! {
                _ = self.heartbeat.tick() => {
                    if self.awaiting_ack { self.ended = true; return Err(Error::HeartbeatTimeout); }
                    self.socket.send(Message::text(r#"{"type":"heartbeat"}"#)).await.map_err(|_| Error::Transport)?;
                    self.awaiting_ack = true;
                }
                frame = self.socket.next() => {
                    match frame {
                        Some(Ok(Message::Text(text))) => return self.decode(text.as_bytes()).map(Some),
                        Some(Ok(Message::Binary(bytes))) => return self.decode(&bytes).map(Some),
                        Some(Ok(Message::Ping(_))) => self.socket.flush().await.map_err(|_| Error::Transport)?,
                        Some(Ok(Message::Pong(_) | Message::Frame(_))) => {},
                        Some(Ok(Message::Close(frame))) => {
                            self.ended = true;
                            let _ = self.socket.flush().await;
                            return Ok(Some(Event::Closed(frame.map_or(CloseReason::NoStatus, |f| u16::from(f.code).into()))));
                        }
                        None => { self.ended = true; return Ok(Some(Event::Closed(CloseReason::Abnormal))); }
                        Some(Err(_)) => { self.ended = true; return Err(Error::Transport); }
                    }
                }
            }
        }
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Event, Error> {
        let event = Event::decode(bytes)?;
        if matches!(event, Event::Known(KnownEvent::HeartbeatAck(_))) {
            self.awaiting_ack = false;
        }
        Ok(event)
    }

    /// Closes this socket explicitly, without changing REST credentials.
    /// # Errors
    /// Returns a sanitized transport failure.
    pub async fn close(&mut self) -> Result<(), Error> {
        self.ended = true;
        self.socket.close(None).await.map_err(|_| Error::Transport)
    }
}

#[cfg(test)]
mod tests {
    //! Owns deterministic decoder coverage and close-code classification without sockets.
    use super::*;

    #[test]
    fn every_backend_event_decodes_and_round_trips() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../protocol/gateway.json")).unwrap();
        for (kind, record) in manifest.as_object().unwrap() {
            if kind.starts_with('_') {
                continue;
            }
            let mut value = serde_json::json!({"type":kind});
            for field in record["fields"].as_array().unwrap() {
                let typ = field[1].as_str().unwrap();
                value[field[0].as_str().unwrap()] = if typ.starts_with('*') {
                    serde_json::Value::Null
                } else if typ.starts_with("[]") {
                    serde_json::json!([])
                } else {
                    match typ {
                        "bool" => serde_json::json!(false),
                        "int" => serde_json::json!(0),
                        "models.DirectConversation" => {
                            serde_json::to_value(crate::wire::DirectConversation::default())
                                .unwrap()
                        }
                        _ => serde_json::json!(""),
                    }
                };
            }
            let event = Event::decode(&serde_json::to_vec(&value).unwrap())
                .unwrap_or_else(|_| panic!("failed kind: {kind}"));
            assert_eq!(event.kind(), kind);
            let Event::Known(known) = event else {
                panic!("known event became unknown");
            };
            assert!(
                serde_json::from_value::<KnownEvent>(serde_json::to_value(known).unwrap()).is_ok()
            );
        }
    }

    #[test]
    fn nil_backend_slices_decode_as_empty_replacement_lists() {
        let event = Event::decode(
            br#"{"type":"message_embeds_update","channel_id":"","message_id":"","embeds":null}"#,
        )
        .unwrap();
        let Event::Known(KnownEvent::MessageEmbedsUpdate(payload)) = event else {
            panic!("wrong event");
        };
        assert!(payload.embeds.is_empty());
        let event =
            Event::decode(br#"{"type":"voice_audio_routes","channel_id":"","routes":null}"#)
                .unwrap();
        assert_eq!(event.kind(), "voice_audio_routes");
    }

    #[test]
    fn missing_discriminators_and_invalid_json_are_rejected() {
        for bytes in [b"{".as_slice(), b"null", b"{}", br#"{"type":4}"#] {
            assert!(Event::decode(bytes).is_err());
        }
    }

    #[test]
    fn close_codes_never_invent_an_authentication_reason() {
        assert_eq!(CloseReason::from(1000), CloseReason::Normal);
        assert_eq!(CloseReason::from(4001), CloseReason::Other(4001));
    }
}
