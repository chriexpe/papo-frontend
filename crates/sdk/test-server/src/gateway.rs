//! Owns the user gateway handshake and deterministic frames; leaves auth, voice and production clients outside.

use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::{HeaderMap, Request, Response};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::create_response,
        protocol::{Role, WebSocketConfig},
    },
};

use crate::{Error, GatewayStep};

pub(crate) fn origin_allowed(headers: &HeaderMap, allowed: &[String]) -> bool {
    headers.get(hyper::header::ORIGIN).is_none_or(|origin| {
        origin.is_empty()
            || origin
                .to_str()
                .is_ok_and(|origin| allowed.iter().any(|allowed| allowed == origin))
    })
}

pub(crate) fn upgrade_response<B>(request: &Request<B>) -> Result<Response<Full<Bytes>>, Error> {
    let mut handshake = Request::new(());
    *handshake.method_mut() = request.method().clone();
    *handshake.uri_mut() = request.uri().clone();
    *handshake.version_mut() = request.version();
    *handshake.headers_mut() = request.headers().clone();
    create_response(&handshake)
        .map(|response| response.map(|()| Full::new(Bytes::new())))
        .map_err(|_| Error::Connection)
}

fn inbound(raw: &[u8]) -> Option<Value> {
    let event = serde_json::from_slice::<Value>(raw).ok();
    match event
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
    {
        Some("heartbeat") => Some(json!({"type": "heartbeat_ack"})),
        Some("presence_activity") => None,
        _ => Some(json!({"type": "error", "message": "evento inválido"})),
    }
}

pub(crate) async fn serve<S>(
    stream: S,
    presence: Value,
    mut updates: tokio::sync::broadcast::Receiver<crate::presence::Update>,
    script: &[GatewayStep],
    ignore_heartbeats: bool,
    mut disconnects: tokio::sync::broadcast::Receiver<()>,
    user_id: &str,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = WebSocketConfig::default().max_message_size(Some(128 * 1024));
    let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, Some(config)).await;
    if socket
        .send(Message::text(presence.to_string()))
        .await
        .is_err()
    {
        return;
    }
    let mut steps = script.iter();
    let mut delay = None;
    let mut waiting_pong = false;
    let mut next_ping = tokio::time::Instant::now() + Duration::from_secs(54);
    let mut pong_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if delay.is_none() && !waiting_pong {
            match steps.next() {
                Some(GatewayStep::Delay { delay_ms }) => {
                    delay = Some(tokio::time::Instant::now() + Duration::from_millis(*delay_ms));
                }
                Some(GatewayStep::Event { event }) => {
                    if socket.send(Message::text(event.to_string())).await.is_err() {
                        return;
                    }
                    continue;
                }
                Some(GatewayStep::Ping) => {
                    if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                        return;
                    }
                    waiting_pong = true;
                }
                Some(GatewayStep::Raw { data }) => {
                    if socket
                        .send(Message::Binary(data.clone().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                Some(GatewayStep::Close { code }) => {
                    let frame = tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: (*code).into(),
                        reason: "".into(),
                    };
                    let _ = socket.close(Some(frame)).await;
                    return;
                }
                Some(GatewayStep::Disconnect) => {
                    let _ = socket.close(None).await;
                    return;
                }
                None => {}
            }
        }
        let wake_at = delay
            .unwrap_or(pong_deadline)
            .min(next_ping)
            .min(pong_deadline);
        tokio::select! {
            _ = disconnects.recv() => return,
            update = updates.recv() => {
                let Ok(update) = update else { let _ = socket.close(None).await; return; };
                if update.recipient.as_deref().is_none_or(|recipient| recipient == user_id) && socket.send(Message::text(update.payload.to_string())).await.is_err() { return; }
            }
            () = tokio::time::sleep_until(wake_at) => {
                let now = tokio::time::Instant::now();
                if now >= pong_deadline { let _ = socket.close(None).await; return; }
                if now >= next_ping {
                    if socket.send(Message::Ping(Bytes::new())).await.is_err() { return; }
                    next_ping = now + Duration::from_secs(54);
                }
                if delay.is_some_and(|deadline| now >= deadline) { delay = None; }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(reply) = reply(text.as_bytes(), ignore_heartbeats)
                            && socket.send(Message::text(reply.to_string())).await.is_err() { return; }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if let Some(reply) = reply(&bytes, ignore_heartbeats)
                            && socket.send(Message::text(reply.to_string())).await.is_err() { return; }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        waiting_pong = false;
                        pong_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
                    },
                    Some(Ok(Message::Ping(_))) => { if socket.flush().await.is_err() { return; } }
                    Some(Ok(Message::Close(_))) => { let _ = socket.flush().await; return; }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(_)) | None => return,
                }
            }
        }
    }
}

fn reply(raw: &[u8], ignore_heartbeats: bool) -> Option<Value> {
    let reply = inbound(raw)?;
    if ignore_heartbeats && reply["type"] == "heartbeat_ack" {
        None
    } else {
        Some(reply)
    }
}

#[cfg(test)]
mod tests {
    //! Owns pure inbound-event tests; leaves socket frames and timing to integration tests.

    use super::*;

    #[test]
    fn heartbeat_has_only_the_verified_acknowledgement_type() {
        let reply = inbound(br#"{"type":"heartbeat"}"#).unwrap();
        assert_eq!(reply, json!({"type": "heartbeat_ack"}));
    }

    #[test]
    fn invalid_json_and_unknown_event_have_the_backend_error() {
        let expected = json!({"type": "error", "message": "evento inválido"});
        assert_eq!(
            (inbound(b"{"), inbound(br#"{"type":"unknown"}"#)),
            (Some(expected.clone()), Some(expected))
        );
    }

    #[test]
    fn activity_has_no_acknowledgement() {
        assert_eq!(inbound(br#"{"type":"presence_activity"}"#), None);
    }

    #[test]
    fn non_browser_origins_need_no_allowlist() {
        assert!(origin_allowed(&HeaderMap::new(), &[]));
    }
}
