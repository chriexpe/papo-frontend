//! Conexão WebSocket: eventos em tempo real do servidor.
//!
//! O handshake usa o mesmo cookie `Auth` da API REST. A conexão se
//! restabelece sozinha com espera progressiva, e um heartbeat sai a cada 30
//! segundos para o servidor não derrubar a sessão.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::client::{Api, Session};
use super::models::Message;

const HEARTBEAT: Duration = Duration::from_secs(30);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Eventos que a interface consome. O que não está aqui é ignorado de
/// propósito — o contrato tem eventos de voz e moderação ainda não usados.
///
/// Alguns campos ainda não têm leitor; ficam aqui porque são o que o servidor
/// manda e o que as telas seguintes vão precisar.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    Message(Box<Message>),
    MessageEdited {
        id: String,
        channel_id: String,
        content: String,
    },
    MessageDeleted {
        id: String,
        channel_id: String,
    },
    MessagePinned {
        message_id: String,
        pinned: bool,
    },
    /// A moderação assíncrona decidiu sobre uma imagem.
    AttachmentModeration {
        message_id: String,
        attachment_id: String,
        status: String,
    },
    Typing {
        channel_id: String,
        user_id: String,
        is_typing: bool,
    },
    Presence {
        user_id: String,
        status: String,
        nickname: Option<String>,
    },
    PresenceSync(Vec<(String, String)>),
    ChannelCreated {
        id: String,
        name: String,
        kind: String,
        topic: Option<String>,
    },
    ChannelUpdated {
        id: String,
        name: String,
        topic: Option<String>,
    },
    ChannelDeleted {
        id: String,
    },
    UserJoined {
        user_id: String,
    },
    Reaction {
        message_id: String,
        unicode: Option<String>,
        emoji_id: Option<String>,
        count: i64,
    },
    /// O evento é unicast e não traz o canal: só o id da mensagem e um
    /// trecho do conteúdo.
    Notification {
        id: String,
        message_id: Option<String>,
        author_id: Option<String>,
        preview: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct PresenceMemberPayload {
    user_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct PresenceSyncPayload {
    #[serde(default)]
    members: Vec<PresenceMemberPayload>,
}

/// Mantém a conexão viva até o canal de saída fechar.
pub async fn run(
    api: Api,
    session: Arc<Session>,
    events: mpsc::UnboundedSender<Event>,
    status: mpsc::UnboundedSender<Connection>,
    mut outbound: mpsc::UnboundedReceiver<String>,
) {
    let mut backoff = BACKOFF_MIN;
    loop {
        if events.is_closed() {
            return;
        }
        let _ = status.send(Connection::Connecting);

        match connect(&api, &session, &events, &status, &mut outbound).await {
            Ok(()) => {
                backoff = BACKOFF_MIN;
                log::info!("websocket encerrado pelo servidor; reconectando");
            }
            Err(error) => {
                log::warn!("websocket caiu: {error}");
            }
        }

        let _ = status.send(Connection::Offline);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn connect(
    api: &Api,
    session: &Session,
    events: &mpsc::UnboundedSender<Event>,
    status: &mpsc::UnboundedSender<Connection>,
    outbound: &mut mpsc::UnboundedReceiver<String>,
) -> Result<(), String> {
    let token = session.token().ok_or_else(|| "sem sessão".to_owned())?;
    let mut request = api
        .websocket_url()
        .into_client_request()
        .map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        "Cookie",
        format!("Auth={token}")
            .parse()
            .map_err(|_| "cookie inválido".to_owned())?,
    );

    let (stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
    log::info!("websocket conectado");
    let _ = status.send(Connection::Online);
    let (mut sink, mut source) = stream.split();

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await; // o primeiro tick sai na hora

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if sink
                    .send(WsMessage::Text(r#"{"type":"heartbeat"}"#.into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            message = outbound.recv() => {
                let Some(message) = message else { return Ok(()) };
                if sink.send(WsMessage::Text(message.into())).await.is_err() {
                    return Ok(());
                }
            }
            incoming = source.next() => {
                let Some(incoming) = incoming else { return Ok(()) };
                let message = incoming.map_err(|e| e.to_string())?;
                match message {
                    WsMessage::Text(text) => {
                        if let Some(event) = parse(&text) {
                            if events.send(event).is_err() {
                                return Ok(());
                            }
                        }
                    }
                    WsMessage::Ping(payload) => {
                        let _ = sink.send(WsMessage::Pong(payload)).await;
                    }
                    WsMessage::Close(_) => return Ok(()),
                    _ => {}
                }
            }
        }
    }
}

/// Traduz o JSON cru para os eventos da interface.
fn parse(text: &str) -> Option<Event> {
    let envelope: Envelope = serde_json::from_str(text).ok()?;
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let string = |key: &str| value.get(key)?.as_str().map(str::to_owned);

    match envelope.kind.as_str() {
        "message" => serde_json::from_str::<Message>(text)
            .ok()
            .map(|message| Event::Message(Box::new(message))),
        "message_edit" => Some(Event::MessageEdited {
            id: string("id")?,
            channel_id: string("channel_id")?,
            content: string("content").unwrap_or_default(),
        }),
        "message_delete" => Some(Event::MessageDeleted {
            id: string("id")?,
            channel_id: string("channel_id")?,
        }),
        "typing" => Some(Event::Typing {
            channel_id: string("channel_id")?,
            user_id: string("user_id")?,
            is_typing: value
                .get("is_typing")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        }),
        "presence_update" => Some(Event::Presence {
            user_id: string("user_id")?,
            status: string("status").unwrap_or_else(|| "offline".to_owned()),
            nickname: string("nickname"),
        }),
        "presence_sync" => {
            let payload: PresenceSyncPayload = serde_json::from_str(text).ok()?;
            Some(Event::PresenceSync(
                payload
                    .members
                    .into_iter()
                    .map(|member| (member.user_id, member.status))
                    .collect(),
            ))
        }
        "channel_create" => Some(Event::ChannelCreated {
            id: string("channel_id")?,
            name: string("name").unwrap_or_default(),
            kind: string("channel_type").unwrap_or_else(|| "text".to_owned()),
            topic: string("topic"),
        }),
        "channel_update" => Some(Event::ChannelUpdated {
            id: string("channel_id")?,
            name: string("name").unwrap_or_default(),
            topic: string("topic"),
        }),
        "channel_delete" => Some(Event::ChannelDeleted {
            id: string("channel_id")?,
        }),
        "user_join" => Some(Event::UserJoined {
            user_id: string("user_id")?,
        }),
        "react_update" => Some(Event::Reaction {
            message_id: string("message_id")?,
            unicode: string("unicode"),
            emoji_id: string("emoji_id"),
            count: value
                .get("count")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
        }),
        "message_pin" => Some(Event::MessagePinned {
            message_id: string("message_id")?,
            pinned: value
                .get("is_pinned")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        }),
        "attachment_moderation_update" => Some(Event::AttachmentModeration {
            message_id: string("message_id")?,
            attachment_id: string("attachment_id")?,
            status: string("status").unwrap_or_default(),
        }),
        "new_notification" => Some(Event::Notification {
            id: string("id").unwrap_or_default(),
            message_id: string("message_id"),
            // `user_id` é o autor da mensagem notificada.
            author_id: string("user_id"),
            preview: string("message_content"),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_evento_de_digitacao() {
        let event = parse(r#"{"type":"typing","channel_id":"c1","user_id":"u1","is_typing":true}"#);
        assert!(matches!(
            event,
            Some(Event::Typing {
                is_typing: true,
                ..
            })
        ));
    }

    #[test]
    fn ignora_evento_desconhecido() {
        assert!(parse(r#"{"type":"voice_offer","sdp":"..."}"#).is_none());
    }
}
