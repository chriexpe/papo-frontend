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
use super::models::{LinkPreview, Message};

const HEARTBEAT: Duration = Duration::from_secs(30);
/// Depois de mandar um heartbeat o backend responde com heartbeat_ack. Sem
/// essa confirmação o TCP pode parecer aberto depois de troca de rede/suspensão.
const HEARTBEAT_ACK_TIMEOUT: Duration = Duration::from_secs(10);
const WATCHDOG_TICK: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
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
    NewPreview {
        message_id: String,
        preview_id: String,
    },
    RemovePreview {
        message_id: String,
        preview_id: String,
    },
    LinkPreviewUpdated {
        message_id: String,
        preview: LinkPreview,
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
    /// Entrada aceita na sala de voz: o estado que o recém-chegado precisa
    /// para desenhar a call antes de qualquer mídia.
    VoiceJoined {
        channel_id: String,
        members: Vec<VoiceMember>,
        active_speakers: Vec<String>,
    },
    /// Resposta SDP do servidor à nossa oferta.
    VoiceAnswer {
        channel_id: String,
        sdp: String,
    },
    /// Oferta do servidor (renegociação que parte dele).
    VoiceOffer {
        channel_id: String,
        sdp: String,
    },
    VoiceCandidate {
        channel_id: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    },
    /// Microfone, câmera ou tela de alguém mudaram. Chega para quem está na
    /// call e para quem só está olhando o canal.
    VoiceState {
        channel_id: String,
        state: VoiceMember,
    },
    VoiceLeft {
        channel_id: String,
        user_id: String,
    },
    /// Quem está falando agora, do mais alto para o mais baixo.
    ActiveSpeakers {
        channel_id: String,
        user_ids: Vec<String>,
    },
    /// Erro do servidor. O `code` é o que distingue um erro de voz
    /// (`voice-room-full`, `voice-forbidden`…) de um aviso qualquer.
    Failure {
        message: String,
        code: Option<String>,
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

/// Estado de uma pessoa na call, como o servidor o publica.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceMember {
    pub user_id: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub camera_on: bool,
    #[serde(default)]
    pub screen_sharing: bool,
}

#[derive(Debug, Deserialize)]
struct VoiceJoinedPayload {
    channel_id: String,
    #[serde(default)]
    members: Option<Vec<VoiceMember>>,
    #[serde(default)]
    active_speakers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ActiveSpeakerPayload {
    channel_id: String,
    #[serde(default)]
    user_ids: Option<Vec<String>>,
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
    mut probe: mpsc::UnboundedReceiver<()>,
) {
    let endpoint = api.websocket_url();
    let mut backoff = BACKOFF_MIN;
    loop {
        if events.is_closed() {
            return;
        }
        let _ = status.send(Connection::Connecting);

        match connect(
            &api,
            &session,
            &events,
            &status,
            &mut outbound,
            &mut probe,
        )
        .await
        {
            Ok(()) => {
                backoff = BACKOFF_MIN;
                log::info!("websocket {endpoint}: encerrado; reconectando");
            }
            Err(error) => {
                log::warn!("websocket {endpoint}: caiu: {error}");
            }
        }

        let _ = status.send(Connection::Offline);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
            signal = probe.recv() => {
                if signal.is_none() {
                    return;
                }
                // Voltar ao foreground não deve esperar um backoff antigo.
                backoff = BACKOFF_MIN;
                log::debug!("websocket {endpoint}: nova tentativa antecipada");
            }
        }
    }
}

async fn connect(
    api: &Api,
    session: &Session,
    events: &mpsc::UnboundedSender<Event>,
    status: &mpsc::UnboundedSender<Connection>,
    outbound: &mut mpsc::UnboundedReceiver<String>,
    probe: &mut mpsc::UnboundedReceiver<()>,
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

    let (stream, _) = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "timeout no handshake".to_owned())?
    .map_err(|e| e.to_string())?;
    log::info!("websocket {}: conectado", api.websocket_url());
    let _ = status.send(Connection::Online);
    let (mut sink, mut source) = stream.split();

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await; // o primeiro tick sai na hora
    let mut watchdog = tokio::time::interval(WATCHDOG_TICK);
    watchdog.tick().await;
    let mut awaiting_ack: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if awaiting_ack.is_none() {
                    if sink
                        .send(WsMessage::Text(r#"{"type":"heartbeat"}"#.into()))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    awaiting_ack = Some(tokio::time::Instant::now());
                }
            }
            _ = watchdog.tick() => {
                if awaiting_ack
                    .is_some_and(|since| since.elapsed() >= HEARTBEAT_ACK_TIMEOUT)
                {
                    return Err("heartbeat sem confirmação".to_owned());
                }
            }
            signal = probe.recv() => {
                let Some(()) = signal else { return Ok(()) };
                // Ao voltar ao foreground, testa a conexão que já existe sem
                // derrubá-la. Se estiver saudável, o ack chega e nada muda;
                // se virou um TCP zumbi, o watchdog a substitui.
                if awaiting_ack.is_none() {
                    if sink
                        .send(WsMessage::Text(r#"{"type":"heartbeat"}"#.into()))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    awaiting_ack = Some(tokio::time::Instant::now());
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
                        if is_heartbeat_ack(&text) {
                            awaiting_ack = None;
                            continue;
                        }
                        if let Some(event) = parse(&text)
                            && events.send(event).is_err()
                        {
                            return Ok(());
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

fn is_heartbeat_ack(text: &str) -> bool {
    serde_json::from_str::<Envelope>(text)
        .is_ok_and(|envelope| envelope.kind == "heartbeat_ack")
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
        "new_preview" => Some(Event::NewPreview {
            message_id: string("message_id")?,
            preview_id: string("preview_id")?,
        }),
        "remove_preview" => Some(Event::RemovePreview {
            message_id: string("message_id")?,
            preview_id: string("preview_id")?,
        }),
        "link_preview_update" => Some(Event::LinkPreviewUpdated {
            message_id: string("message_id")?,
            preview: serde_json::from_value(value.get("preview")?.clone()).ok()?,
        }),
        "attachment_moderation_update" => Some(Event::AttachmentModeration {
            message_id: string("message_id")?,
            attachment_id: string("attachment_id")?,
            status: string("status").unwrap_or_default(),
        }),
        "voice_joined" => {
            let payload: VoiceJoinedPayload = serde_json::from_str(text).ok()?;
            Some(Event::VoiceJoined {
                channel_id: payload.channel_id,
                members: payload.members.unwrap_or_default(),
                active_speakers: payload.active_speakers.unwrap_or_default(),
            })
        }
        "voice_answer" => Some(Event::VoiceAnswer {
            channel_id: string("channel_id")?,
            sdp: string("sdp")?,
        }),
        "voice_offer" => Some(Event::VoiceOffer {
            channel_id: string("channel_id")?,
            sdp: string("sdp")?,
        }),
        "voice_ice_candidate" => Some(Event::VoiceCandidate {
            channel_id: string("channel_id")?,
            candidate: string("candidate")?,
            sdp_mid: string("sdp_mid"),
            sdp_mline_index: value
                .get("sdp_mline_index")
                .and_then(serde_json::Value::as_u64)
                .map(|index| index as u32),
        }),
        "voice_state_update" => Some(Event::VoiceState {
            channel_id: string("channel_id")?,
            state: serde_json::from_str(text).ok()?,
        }),
        "voice_leave" => Some(Event::VoiceLeft {
            channel_id: string("channel_id")?,
            user_id: string("user_id")?,
        }),
        "active_speaker_update" => {
            let payload: ActiveSpeakerPayload = serde_json::from_str(text).ok()?;
            Some(Event::ActiveSpeakers {
                channel_id: payload.channel_id,
                user_ids: payload.user_ids.unwrap_or_default(),
            })
        }
        "error" => Some(Event::Failure {
            message: string("message").unwrap_or_default(),
            code: string("code"),
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
        assert!(parse(r#"{"type":"coisa_nova","sdp":"..."}"#).is_none());
    }

    #[test]
    fn heartbeat_ack_e_controle_do_socket_nao_vira_evento_de_ui() {
        assert!(is_heartbeat_ack(r#"{"type":"heartbeat_ack"}"#));
        assert!(!is_heartbeat_ack(r#"{"type":"message"}"#));
        assert!(parse(r#"{"type":"heartbeat_ack"}"#).is_none());
    }

    /// O `voice_joined` chega com `members: null` quando a sala está vazia;
    /// sem tolerar isso a entrada na call morria calada.
    #[test]
    fn entrada_na_call_aceita_lista_nula() {
        let event = parse(r#"{"type":"voice_joined","channel_id":"c1","members":null,"active_speakers":null}"#);
        assert!(matches!(
            event,
            Some(Event::VoiceJoined { ref members, .. }) if members.is_empty()
        ));
    }

    #[test]
    fn le_o_candidato_de_ice() {
        let event = parse(
            r#"{"type":"voice_ice_candidate","channel_id":"c1","candidate":"candidate:1 1 udp 2130706431 192.168.0.114 50000 typ host","sdp_mid":"0","sdp_mline_index":0}"#,
        );
        assert!(matches!(
            event,
            Some(Event::VoiceCandidate {
                sdp_mline_index: Some(0),
                ..
            })
        ));
    }
}
