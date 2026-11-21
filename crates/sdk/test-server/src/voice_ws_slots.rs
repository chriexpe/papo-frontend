//! Video slot subscription and `voice_video_routes` emission for `TestServer`.
//! General voice join/leave/SDP/device handlers live in `voice_ws.rs`.

use serde_json::{Value, json};

use crate::{
    presence::Presence,
    server::State,
    voice::{VoiceFault, VoicePeer},
};

fn voice_error(code: &str) -> Value {
    json!({"type": "error", "message": code, "code": code})
}

pub(crate) fn handle_subscribe(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    event: &Value,
) -> Option<Value> {
    let Some(publisher_id) = event
        .get("publisher_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Some(json!({"type": "error", "message": "evento inválido"}));
    };
    let Some(kind) = event
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| matches!(*k, "video" | "screen"))
    else {
        return Some(json!({"type": "error", "message": "evento inválido"}));
    };
    if let Some(VoiceFault::RejectNextSubscribe(code)) = state
        .voice
        .take_fault(|f| matches!(f, VoiceFault::RejectNextSubscribe(_)))
    {
        return Some(voice_error(&code));
    }
    let Some(room) = state.voice.rooms.get_mut(channel_id) else {
        return Some(voice_error("voice-not-found"));
    };
    let publisher_active = room.peers.get(publisher_id).is_some_and(|pub_peer| {
        if kind == "video" {
            pub_peer.camera_on
        } else {
            pub_peer.screen_sharing
        }
    });
    if !publisher_active || publisher_id == user_id {
        return Some(voice_error("voice-not-found"));
    }
    let Some(sub) = room
        .peers
        .get_mut(user_id)
        .filter(|peer| peer.client_id == client_id)
    else {
        return Some(voice_error("voice-not-found"));
    };
    if sub
        .video_slots
        .iter()
        .flatten()
        .any(|(pub_id, k, _)| pub_id == publisher_id && k == kind)
    {
        emit_video_routes(&state.presence, channel_id, sub);
        return None;
    }
    let Some(index) = sub.video_slots.iter().position(Option::is_none) else {
        return Some(voice_error("voice-room-full"));
    };
    let Some(assignment_id) = sub.routes_revision.checked_add(1) else {
        return Some(voice_error("voice-room-full"));
    };
    sub.video_slots[index] = Some((publisher_id.to_owned(), kind.to_owned(), assignment_id));
    emit_video_routes(&state.presence, channel_id, sub);
    None
}

pub(crate) fn handle_unsubscribe(
    state: &mut State,
    user_id: &str,
    client_id: &str,
    channel_id: &str,
    event: &Value,
) -> Option<Value> {
    let Some(publisher_id) = event.get("publisher_id").and_then(Value::as_str) else {
        return Some(json!({"type": "error", "message": "evento inválido"}));
    };
    let Some(kind) = event.get("kind").and_then(Value::as_str) else {
        return Some(json!({"type": "error", "message": "evento inválido"}));
    };
    let Some(room) = state.voice.rooms.get_mut(channel_id) else {
        return Some(voice_error("voice-not-found"));
    };
    let Some(sub) = room
        .peers
        .get_mut(user_id)
        .filter(|peer| peer.client_id == client_id)
    else {
        return Some(voice_error("voice-not-found"));
    };
    let mut changed = false;
    for slot in &mut sub.video_slots {
        if slot
            .as_ref()
            .is_some_and(|(pub_id, k, _)| pub_id == publisher_id && k == kind)
        {
            *slot = None;
            changed = true;
        }
    }
    if changed {
        emit_video_routes(&state.presence, channel_id, sub);
    }
    None
}

fn emit_video_routes(presence: &Presence, channel_id: &str, sub: &mut VoicePeer) {
    sub.routes_revision = sub.routes_revision.saturating_add(1);
    let routes: Vec<Value> = sub
        .video_slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            let (pub_id, kind, assignment_id) = slot.as_ref()?;
            Some(json!({
                "track_id": format!("papo-video-{index}"),
                "user_id": pub_id,
                "kind": kind,
                "rtp_route_id": assignment_id,
                "rtp_route_ext_id": 14,
            }))
        })
        .collect();
    presence.emit_to_client(
        &sub.client_id,
        json!({
            "type": "voice_video_routes",
            "channel_id": channel_id,
            "peer_id": sub.peer_id,
            "revision": sub.routes_revision,
            "routes": routes,
        }),
    );
}
