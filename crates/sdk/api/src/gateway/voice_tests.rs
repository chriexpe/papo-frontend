//! Typed encode/decode fixtures for 14-0 voice gateway events and voice-scoped errors.

use super::{Event, KnownEvent};
use serde_json::json;

#[test]
fn voice_joined_decodes_peer_id_codec_members_and_speakers() {
    let payload = json!({
        "type": "voice_joined",
        "channel_id": "chan-1",
        "peer_id": "peer-abc",
        "video_codec": "h264",
        "members": [{
            "user_id": "user-1",
            "muted": true,
            "camera_on": false,
            "screen_sharing": true
        }],
        "active_speakers": ["user-1"]
    });
    let Event::Known(KnownEvent::VoiceJoined(event)) =
        Event::decode(payload.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_joined");
    };
    assert_eq!(event.channel_id, "chan-1");
    assert_eq!(event.peer_id, "peer-abc");
    assert_eq!(event.video_codec, "h264");
    assert_eq!(event.members.len(), 1);
    assert_eq!(event.members[0].user_id, "user-1");
    assert!(event.members[0].muted);
    assert!(!event.members[0].camera_on);
    assert!(event.members[0].screen_sharing);
    assert_eq!(event.active_speakers, vec!["user-1"]);
}

#[test]
fn voice_sdp_and_ice_events_preserve_peer_id_and_optional_candidate_fields() {
    let answer = json!({
        "type": "voice_answer",
        "channel_id": "c1",
        "peer_id": "p1",
        "sdp": "v=0\r\n"
    });
    let Event::Known(KnownEvent::VoiceAnswer(event)) =
        Event::decode(answer.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_answer");
    };
    assert_eq!(event.channel_id, "c1");
    assert_eq!(event.peer_id, "p1");
    assert_eq!(event.sdp, "v=0\r\n");

    let offer = json!({
        "type": "voice_offer",
        "channel_id": "c1",
        "peer_id": "p1",
        "sdp": "v=0\r\n"
    });
    let Event::Known(KnownEvent::VoiceOffer(event)) =
        Event::decode(offer.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_offer");
    };
    assert_eq!(event.peer_id, "p1");

    let ice_with_mid = json!({
        "type": "voice_ice_candidate",
        "channel_id": "c1",
        "peer_id": "p1",
        "candidate": "candidate:1",
        "sdp_mid": "0",
        "sdp_mline_index": 0
    });
    let Event::Known(KnownEvent::VoiceICECandidate(event)) =
        Event::decode(ice_with_mid.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_ice_candidate");
    };
    assert_eq!(event.peer_id, "p1");
    assert_eq!(event.sdp_mid.as_deref(), Some("0"));
    assert_eq!(event.sdp_mline_index, Some(0));

    let ice_minimal = json!({
        "type": "voice_ice_candidate",
        "channel_id": "c1",
        "peer_id": "p1",
        "candidate": "candidate:2"
    });
    let Event::Known(KnownEvent::VoiceICECandidate(event)) =
        Event::decode(ice_minimal.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_ice_candidate");
    };
    assert!(event.sdp_mid.is_none());
    assert!(event.sdp_mline_index.is_none());
}

#[test]
fn voice_leave_state_update_and_speakers_decode_all_fields() {
    let leave = json!({
        "type": "voice_leave",
        "channel_id": "c1",
        "user_id": "u1",
        "peer_id": "p-old"
    });
    let Event::Known(KnownEvent::VoiceLeave(event)) =
        Event::decode(leave.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_leave");
    };
    assert_eq!(event.channel_id, "c1");
    assert_eq!(event.user_id, "u1");
    assert_eq!(event.peer_id, "p-old");

    let state = json!({
        "type": "voice_state_update",
        "channel_id": "c1",
        "user_id": "u1",
        "muted": false,
        "camera_on": true,
        "screen_sharing": false
    });
    let Event::Known(KnownEvent::VoiceStateUpdate(event)) =
        Event::decode(state.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_state_update");
    };
    assert!(!event.muted);
    assert!(event.camera_on);
    assert!(!event.screen_sharing);

    let speakers = json!({
        "type": "active_speaker_update",
        "channel_id": "c1",
        "user_ids": ["u1", "u2"]
    });
    let Event::Known(KnownEvent::ActiveSpeakerUpdate(event)) =
        Event::decode(speakers.to_string().as_bytes()).unwrap()
    else {
        panic!("expected active_speaker_update");
    };
    assert_eq!(event.user_ids, vec!["u1", "u2"]);
}

#[test]
fn voice_audio_and_video_routes_decode_revisions_and_optional_fences() {
    let audio = json!({
        "type": "voice_audio_routes",
        "channel_id": "c1",
        "peer_id": "p1",
        "revision": 7,
        "routes": [{"track_id": "papo-audio-0", "user_id": "u1"}]
    });
    let Event::Known(KnownEvent::VoiceAudioRoutes(event)) =
        Event::decode(audio.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_audio_routes");
    };
    assert_eq!(event.peer_id, "p1");
    assert_eq!(event.revision, 7);
    assert_eq!(event.routes.len(), 1);
    assert_eq!(event.routes[0].track_id, "papo-audio-0");
    assert_eq!(event.routes[0].user_id, "u1");

    let video = json!({
        "type": "voice_video_routes",
        "channel_id": "c1",
        "peer_id": "p1",
        "revision": 8,
        "routes": [
            {"track_id": "papo-video-0", "user_id": "u1", "kind": "video"},
            {"track_id": "papo-video-1", "user_id": "u2", "kind": "screen", "first_seq": 65535, "rtp_route_id": u64::MAX, "rtp_route_ext_id": 14},
            {"track_id": "papo-video-2", "user_id": "u3", "kind": "video", "rtp_route_id": "bad", "rtp_route_ext_id": 256}
        ]
    });
    let Event::Known(KnownEvent::VoiceVideoRoutes(event)) =
        Event::decode(video.to_string().as_bytes()).unwrap()
    else {
        panic!("expected voice_video_routes");
    };
    assert_eq!(event.peer_id, "p1");
    assert_eq!(event.revision, 8);
    assert_eq!(event.routes.len(), 3);
    assert_eq!(event.routes[0].track_id, "papo-video-0");
    assert_eq!(event.routes[0].kind, "video");
    assert_eq!(event.routes[0].first_seq, None);
    assert_eq!(event.routes[1].track_id, "papo-video-1");
    assert_eq!(event.routes[1].kind, "screen");
    assert_eq!(event.routes[1].first_seq, Some(65535));
    assert_eq!(event.routes[0].rtp_route_id, None);
    assert_eq!(event.routes[0].rtp_route_ext_id, None);
    assert_eq!(event.routes[1].rtp_route_id, Some(u64::MAX));
    assert_eq!(event.routes[1].rtp_route_ext_id, Some(14));
    assert_eq!(event.routes[2].rtp_route_id, None);
    assert_eq!(event.routes[2].rtp_route_ext_id, None);
}

#[test]
fn voice_error_event_decodes_with_and_without_channel_and_peer_scope() {
    let direct = json!({
        "type": "error",
        "message": "sala de voz cheia",
        "code": "voice-room-full"
    });
    let Event::Known(KnownEvent::Error(event)) =
        Event::decode(direct.to_string().as_bytes()).unwrap()
    else {
        panic!("expected error");
    };
    assert_eq!(event.code.as_deref(), Some("voice-room-full"));
    assert!(event.channel_id.is_none());
    assert!(event.peer_id.is_none());

    let scoped = json!({
        "type": "error",
        "message": "SDP inválido",
        "code": "voice-invalid-sdp",
        "channel_id": "c1",
        "peer_id": "p1"
    });
    let Event::Known(KnownEvent::Error(event)) =
        Event::decode(scoped.to_string().as_bytes()).unwrap()
    else {
        panic!("expected error");
    };
    assert_eq!(event.code.as_deref(), Some("voice-invalid-sdp"));
    assert_eq!(event.channel_id.as_deref(), Some("c1"));
    assert_eq!(event.peer_id.as_deref(), Some("p1"));
}
