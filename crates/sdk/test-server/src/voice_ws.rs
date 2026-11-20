//! WebSocket voice signalling handlers for `TestServer`, mirroring the 14-0 backend contracts.
//! Voice state definitions, REST `GET /voice/ice-servers` and `TestServer` controls live in `voice.rs`.

use serde_json::{Value, json};

use crate::{
    server::State,
    voice::{RecordedVoiceCommand, VoiceFault, VoicePeer},
    voice_ws_slots::{handle_subscribe, handle_unsubscribe},
};

/// Outcome of inspecting an inbound WebSocket JSON frame for voice signalling.
pub(crate) enum VoiceWsOutcome {
    /// Not a voice command; let generic gateway handling process it.
    NotVoice,
    /// Handled as a voice command with no direct error reply.
    Handled,
    /// Handled as a voice command returning a direct error frame to this client.
    Error(Value),
}

impl From<Option<Value>> for VoiceWsOutcome {
    fn from(error: Option<Value>) -> Self {
        match error {
            Some(frame) => Self::Error(frame),
            None => Self::Handled,
        }
    }
}

/// Handles an inbound voice WebSocket command.
pub(crate) fn handle_ws(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    event: &Value,
) -> VoiceWsOutcome {
    let Some(kind) = event.get("type").and_then(Value::as_str) else {
        return VoiceWsOutcome::NotVoice;
    };
    if !matches!(
        kind,
        "voice_join"
            | "voice_leave"
            | "voice_offer"
            | "voice_answer"
            | "voice_ice_candidate"
            | "track_subscribe"
            | "track_unsubscribe"
            | "voice_mute"
            | "voice_camera"
            | "screen_share_start"
            | "screen_share_stop"
    ) {
        return VoiceWsOutcome::NotVoice;
    }
    let Some(channel_id) = event
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return VoiceWsOutcome::Error(json!({"type": "error", "message": "evento inválido"}));
    };

    state.voice.recorded.push(RecordedVoiceCommand {
        user_id: user_id.to_owned(),
        client_id: client_id.to_owned(),
        kind: kind.to_owned(),
        channel_id: channel_id.to_owned(),
    });

    match kind {
        "voice_join" => handle_join(state, user_id, client_id, channel_id).into(),
        "voice_leave" => handle_leave(state, user_id, client_id, channel_id).into(),
        "voice_offer" => handle_offer(state, user_id, client_id, channel_id, event).into(),
        "voice_answer" | "voice_ice_candidate" => {
            handle_owner_only(state, user_id, client_id, channel_id).into()
        }
        "voice_mute" => handle_mute(state, user_id, client_id, channel_id, event).into(),
        "voice_camera" => handle_camera(state, user_id, client_id, channel_id, event).into(),
        "screen_share_start" => handle_screen(state, user_id, client_id, channel_id, true).into(),
        "screen_share_stop" => handle_screen(state, user_id, client_id, channel_id, false).into(),
        "track_subscribe" => handle_subscribe(state, user_id, client_id, channel_id, event).into(),
        "track_unsubscribe" => {
            handle_unsubscribe(state, user_id, client_id, channel_id, event).into()
        }
        _ => VoiceWsOutcome::NotVoice,
    }
}

fn voice_error(code: &str) -> Value {
    json!({"type": "error", "message": code, "code": code})
}

fn handle_join(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
) -> Option<Value> {
    let Some(channel) = state.resources.channels.get(channel_id) else {
        return Some(json!({"type": "error", "message": "canal não encontrado"}));
    };
    if channel.r#type.as_deref() != Some("voice") {
        return Some(voice_error("voice-forbidden"));
    }
    if let Some(VoiceFault::RejectNextJoin(code)) = state
        .voice
        .take_fault(|f| matches!(f, VoiceFault::RejectNextJoin(_)))
    {
        return Some(voice_error(&code));
    }
    if state
        .voice
        .rooms
        .values()
        .any(|room| room.peers.contains_key(user_id))
    {
        return Some(voice_error("voice-already-in-room"));
    }

    let peer_id = state.voice.next_peer_id();
    let codec = state.voice.video_codec.clone();
    let drop_joined = state
        .voice
        .take_fault(|f| matches!(f, VoiceFault::DropNextVoiceJoined))
        .is_some();

    let room = state.voice.rooms.entry(channel_id.to_owned()).or_default();
    room.peers.insert(
        user_id.to_owned(),
        VoicePeer {
            peer_id: peer_id.clone(),
            user_id: user_id.to_owned(),
            client_id: client_id.to_owned(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
            routes_revision: 0,
            video_slots: vec![None; 6],
        },
    );
    let members: Vec<Value> = room
        .peers
        .values()
        .map(|peer| {
            json!({
                "user_id": peer.user_id,
                "muted": peer.muted,
                "camera_on": peer.camera_on,
                "screen_sharing": peer.screen_sharing,
            })
        })
        .collect();
    let speakers = room.active_speakers.clone();

    if !drop_joined {
        state.presence.emit_to_client(
            client_id,
            json!({
                "type": "voice_joined",
                "channel_id": channel_id,
                "peer_id": peer_id,
                "video_codec": codec,
                "members": members,
                "active_speakers": speakers,
            }),
        );
    }
    state.presence.emit(
        None,
        json!({
            "type": "voice_state_update",
            "channel_id": channel_id,
            "user_id": user_id,
            "muted": true,
            "camera_on": false,
            "screen_sharing": false,
        }),
    );
    None
}

fn handle_leave(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
) -> Option<Value> {
    let owns = state
        .voice
        .rooms
        .get(channel_id)
        .and_then(|room| room.peers.get(user_id))
        .is_some_and(|peer| peer.client_id == client_id);
    if !owns {
        return Some(voice_error("voice-not-found"));
    }
    state
        .voice
        .remove_peer(&state.presence, channel_id, user_id);
    None
}

fn handle_owner_only(
    state: &State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
) -> Option<Value> {
    let owns = state
        .voice
        .rooms
        .get(channel_id)
        .and_then(|room| room.peers.get(user_id))
        .is_some_and(|peer| peer.client_id == client_id);
    if owns {
        None
    } else {
        Some(voice_error("voice-not-found"))
    }
}

fn handle_offer(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    event: &Value,
) -> Option<Value> {
    if event
        .get("sdp")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Some(json!({"type": "error", "message": "evento inválido"}));
    }
    let Some(peer_id) = state
        .voice
        .rooms
        .get(channel_id)
        .and_then(|room| room.peers.get(user_id))
        .filter(|peer| peer.client_id == client_id)
        .map(|peer| peer.peer_id.clone())
    else {
        return Some(voice_error("voice-not-found"));
    };
    if state
        .voice
        .take_fault(|f| matches!(f, VoiceFault::DropNextVoiceAnswer))
        .is_none()
    {
        state.presence.emit_to_client(
            client_id,
            json!({
                "type": "voice_answer",
                "channel_id": channel_id,
                "peer_id": peer_id,
                "sdp": "v=0\r\n",
            }),
        );
    }
    None
}

fn handle_mute(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    event: &Value,
) -> Option<Value> {
    let muted = event.get("muted").and_then(Value::as_bool).unwrap_or(false);
    let Some(room) = state.voice.rooms.get_mut(channel_id) else {
        return Some(voice_error("voice-not-found"));
    };
    let Some(peer) = room
        .peers
        .get_mut(user_id)
        .filter(|peer| peer.client_id == client_id)
    else {
        return Some(voice_error("voice-not-found"));
    };
    peer.muted = muted;
    let payload = json!({
        "type": "voice_state_update",
        "channel_id": channel_id,
        "user_id": user_id,
        "muted": peer.muted,
        "camera_on": peer.camera_on,
        "screen_sharing": peer.screen_sharing,
    });
    if muted {
        room.active_speakers.retain(|speaker| speaker != user_id);
    }
    state.presence.emit(None, payload);
    None
}

fn handle_camera(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    event: &Value,
) -> Option<Value> {
    let on = event.get("on").and_then(Value::as_bool).unwrap_or(false);
    let Some(room) = state.voice.rooms.get_mut(channel_id) else {
        return Some(voice_error("voice-not-found"));
    };
    let Some(peer) = room
        .peers
        .get_mut(user_id)
        .filter(|peer| peer.client_id == client_id)
    else {
        return Some(voice_error("voice-not-found"));
    };
    peer.camera_on = on;
    let payload = json!({
        "type": "voice_state_update",
        "channel_id": channel_id,
        "user_id": user_id,
        "muted": peer.muted,
        "camera_on": peer.camera_on,
        "screen_sharing": peer.screen_sharing,
    });
    state.presence.emit(None, payload);
    None
}

fn handle_screen(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    on: bool,
) -> Option<Value> {
    let Some(room) = state.voice.rooms.get_mut(channel_id) else {
        return Some(voice_error("voice-not-found"));
    };
    let Some(peer) = room
        .peers
        .get_mut(user_id)
        .filter(|peer| peer.client_id == client_id)
    else {
        return Some(voice_error("voice-not-found"));
    };
    peer.screen_sharing = on;
    let payload = json!({
        "type": "voice_state_update",
        "channel_id": channel_id,
        "user_id": user_id,
        "muted": peer.muted,
        "camera_on": peer.camera_on,
        "screen_sharing": peer.screen_sharing,
    });
    state.presence.emit(None, payload);
    None
}
