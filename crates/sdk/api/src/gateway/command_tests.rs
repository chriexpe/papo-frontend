//! Tests outbound command wire shapes and handler-level checks without a socket.

use super::{Command, TrackKind};
use crate::gateway::Error;
use serde_json::{Value, json};

fn encoded(command: &Command) -> Value {
    serde_json::from_str(&command.encode().unwrap()).unwrap()
}

#[test]
fn every_client_command_has_the_backend_wire_shape() {
    let c = || String::from("channel");
    let p = || String::from("publisher");
    let cases = [
        (
            Command::PresenceActivity,
            json!({"type":"presence_activity"}),
        ),
        (Command::Heartbeat, json!({"type":"heartbeat"})),
        (
            Command::Typing { channel_id: c() },
            json!({"type":"typing","channel_id":"channel"}),
        ),
        (
            Command::VoiceJoin { channel_id: c() },
            json!({"type":"voice_join","channel_id":"channel"}),
        ),
        (
            Command::VoiceLeave { channel_id: c() },
            json!({"type":"voice_leave","channel_id":"channel"}),
        ),
        (
            Command::VoiceOffer {
                channel_id: c(),
                sdp: "sdp".into(),
            },
            json!({"type":"voice_offer","channel_id":"channel","sdp":"sdp"}),
        ),
        (
            Command::VoiceAnswer {
                channel_id: c(),
                sdp: "sdp".into(),
            },
            json!({"type":"voice_answer","channel_id":"channel","sdp":"sdp"}),
        ),
        (
            Command::VoiceIceCandidate {
                channel_id: c(),
                candidate: "candidate".into(),
                sdp_mid: None,
                sdp_mline_index: None,
            },
            json!({"type":"voice_ice_candidate","channel_id":"channel","candidate":"candidate","sdp_mid":null,"sdp_mline_index":null}),
        ),
        (
            Command::TrackSubscribe {
                channel_id: c(),
                publisher_id: p(),
                kind: TrackKind::Video,
            },
            json!({"type":"track_subscribe","channel_id":"channel","publisher_id":"publisher","kind":"video"}),
        ),
        (
            Command::TrackUnsubscribe {
                channel_id: c(),
                publisher_id: p(),
                kind: TrackKind::Screen,
            },
            json!({"type":"track_unsubscribe","channel_id":"channel","publisher_id":"publisher","kind":"screen"}),
        ),
        (
            Command::VoiceMute {
                channel_id: c(),
                muted: false,
            },
            json!({"type":"voice_mute","channel_id":"channel","muted":false}),
        ),
        (
            Command::VoiceCamera {
                channel_id: c(),
                on: true,
            },
            json!({"type":"voice_camera","channel_id":"channel","on":true}),
        ),
        (
            Command::ScreenShareStart { channel_id: c() },
            json!({"type":"screen_share_start","channel_id":"channel"}),
        ),
        (
            Command::ScreenShareStop { channel_id: c() },
            json!({"type":"screen_share_stop","channel_id":"channel"}),
        ),
    ];
    for (command, expected) in cases {
        assert_eq!(encoded(&command), expected);
    }
}

#[test]
fn ice_optional_fields_preserve_values_and_signed_index() {
    let command = Command::VoiceIceCandidate {
        channel_id: "c".into(),
        candidate: "x".into(),
        sdp_mid: Some(String::new()),
        sdp_mline_index: Some(-1),
    };
    assert_eq!(
        encoded(&command),
        json!({"type":"voice_ice_candidate","channel_id":"c","candidate":"x","sdp_mid":"","sdp_mline_index":-1})
    );
}

#[test]
fn empty_handler_required_fields_are_rejected() {
    let cases = [
        Command::Typing {
            channel_id: String::new(),
        },
        Command::VoiceJoin {
            channel_id: String::new(),
        },
        Command::VoiceLeave {
            channel_id: String::new(),
        },
        Command::VoiceOffer {
            channel_id: String::new(),
            sdp: "s".into(),
        },
        Command::VoiceOffer {
            channel_id: "c".into(),
            sdp: String::new(),
        },
        Command::VoiceAnswer {
            channel_id: String::new(),
            sdp: "s".into(),
        },
        Command::VoiceAnswer {
            channel_id: "c".into(),
            sdp: String::new(),
        },
        Command::VoiceIceCandidate {
            channel_id: String::new(),
            candidate: "x".into(),
            sdp_mid: None,
            sdp_mline_index: None,
        },
        Command::VoiceIceCandidate {
            channel_id: "c".into(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_mline_index: None,
        },
        Command::TrackSubscribe {
            channel_id: String::new(),
            publisher_id: "p".into(),
            kind: TrackKind::Video,
        },
        Command::TrackSubscribe {
            channel_id: "c".into(),
            publisher_id: String::new(),
            kind: TrackKind::Video,
        },
        Command::TrackUnsubscribe {
            channel_id: String::new(),
            publisher_id: "p".into(),
            kind: TrackKind::Screen,
        },
        Command::TrackUnsubscribe {
            channel_id: "c".into(),
            publisher_id: String::new(),
            kind: TrackKind::Screen,
        },
        Command::VoiceMute {
            channel_id: String::new(),
            muted: false,
        },
        Command::VoiceCamera {
            channel_id: String::new(),
            on: false,
        },
        Command::ScreenShareStart {
            channel_id: String::new(),
        },
        Command::ScreenShareStop {
            channel_id: String::new(),
        },
    ];
    for command in cases {
        assert!(matches!(command.encode(), Err(Error::InvalidCommand)));
    }
}

#[test]
fn whitespace_is_not_trimmed_or_rejected() {
    let command = Command::VoiceOffer {
        channel_id: " ".into(),
        sdp: " ".into(),
    };
    assert_eq!(
        encoded(&command),
        json!({"type":"voice_offer","channel_id":" ","sdp":" "})
    );
}
