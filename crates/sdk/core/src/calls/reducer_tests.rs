//! Pure deterministic voice reducer tests, including ported `papo-core/src/state/call.rs` cases.

use super::{
    model::{SpeakingState, VideoCodec, VideoKind, VoiceDeviceState},
    reducer::{ActivePeerRef, VoiceReducer},
};
use crate::{
    ChannelKind, ChannelRecord, Generation, Overwrite, OverwriteKey, ResourceSnapshot,
    ResourceTable, RoleFlags, RoleRecord, ServerRecord, UserRecord, Uuid,
};
use papo_api::gateway::{
    ActiveSpeakerUpdateEvent, PresenceMember, PresenceSyncEvent, VoiceAudioRoute,
    VoiceAudioRoutesEvent, VoiceJoinedEvent, VoiceLeaveEvent, VoiceState, VoiceStateUpdateEvent,
    VoiceVideoRoute, VoiceVideoRoutesEvent,
};
use std::sync::Arc;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn resources_with_channels(
    user: Uuid,
    role: Uuid,
    channels: Vec<ChannelRecord>,
) -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(Generation::for_test(1));
    snapshot.channel_census_complete = true;
    snapshot.server = Some(Arc::new(ServerRecord {
        id: id(1),
        name: None,
        icon_format: None,
        owner_id: Some(id(999)),
        public: Some(true),
        owner_username: None,
        created_at: None,
        channel_count: None,
        member_count: None,
        role_count: None,
    }));
    snapshot.roles = ResourceTable::from_rows([(
        role,
        RoleRecord {
            id: role,
            name: None,
            color: None,
            permissions: RoleFlags {
                manage_server: None,
                manage_channels: None,
                manage_roles: None,
                manage_webhooks: None,
                ban_members: None,
                pin_message: None,
                everyone_message: None,
                send_attachment: None,
            },
            created_at: None,
        },
    )]);
    snapshot.users.set(
        user,
        UserRecord {
            id: user,
            username: Some("me".into()),
            nickname: None,
            banned: None,
            typing: None,
            created_at: None,
            roles: Some(vec![role]),
        },
    );
    snapshot.members.replace([user].into_iter());
    snapshot.channels = ResourceTable::from_rows(channels.into_iter().map(|c| (c.id, c)));
    snapshot
}

fn channel(channel_id: Uuid, kind: ChannelKind, overwrites: Vec<Overwrite>) -> ChannelRecord {
    ChannelRecord {
        id: channel_id,
        name: Some("room".into()),
        kind,
        position: Some(1),
        topic: None,
        parent_id: None,
        permissions: overwrites,
        created_at: None,
        last_message: None,
        last_read_message: None,
        last_read_at: None,
        notification_settings: None,
    }
}

#[test]
fn cold_presence_filters_by_admitted_voice_permission_and_never_invents_device_state() {
    let (me, ana, role) = (id(10), id(11), id(20));
    let (allowed_ch, denied_ch, unknown_ch) = (id(30), id(31), id(32));
    let resources = resources_with_channels(
        me,
        role,
        vec![
            channel(
                allowed_ch,
                ChannelKind::Voice,
                vec![Overwrite {
                    key: OverwriteKey::Canonical(role),
                    read_channel: false,
                    send_messages: Some(false),
                    delete_messages: Some(false),
                    connect_voice: Some(true),
                }],
            ),
            channel(
                denied_ch,
                ChannelKind::Voice,
                vec![Overwrite {
                    key: OverwriteKey::Canonical(role),
                    read_channel: true,
                    send_messages: Some(true),
                    delete_messages: Some(false),
                    connect_voice: Some(false),
                }],
            ),
        ],
    );

    let mut reducer = VoiceReducer::new(Generation::for_test(1));
    reducer.apply_presence_sync(&PresenceSyncEvent {
        members: vec![PresenceMember {
            user_id: ana.to_string(),
            status: "online".into(),
            status_message: None,
            user_voice: vec![
                allowed_ch.to_string(),
                denied_ch.to_string(),
                unknown_ch.to_string(),
            ],
        }],
    });

    let visible = reducer.project_rooms(1, true, &resources, me);
    assert_eq!(visible.rooms.len(), 1);
    let room = visible.rooms.get(&allowed_ch).unwrap();
    assert_eq!(room.members.len(), 1);
    assert_eq!(room.members[0].user, ana);
    assert!(room.members[0].device.is_none());
    assert_eq!(
        room.speaking_state(ana, room.members[0].device),
        SpeakingState::Unknown
    );
    assert!(!visible.rooms.contains_key(&denied_ch));
    assert!(!visible.rooms.contains_key(&unknown_ch));

    // Ported from papo-core: rich live state is preserved across a subsequent presence_sync.
    reducer.apply_state_update(&VoiceStateUpdateEvent {
        channel_id: allowed_ch.to_string(),
        user_id: ana.to_string(),
        muted: false,
        camera_on: true,
        screen_sharing: false,
    });
    reducer.apply_presence_sync(&PresenceSyncEvent {
        members: vec![PresenceMember {
            user_id: ana.to_string(),
            status: "online".into(),
            status_message: None,
            user_voice: vec![allowed_ch.to_string()],
        }],
    });
    let visible = reducer.project_rooms(2, true, &resources, me);
    assert_eq!(
        visible.rooms[&allowed_ch].members[0].device,
        Some(VoiceDeviceState {
            muted: false,
            camera_on: true,
            screen_sharing: false,
        })
    );
}

#[test]
fn state_update_creates_and_leave_removes_while_preserving_other_occupants() {
    let (me, ana, beto) = (id(10), id(11), id(12));
    let (c1, c2) = (id(30), id(31));
    let mut reducer = VoiceReducer::new(Generation::for_test(1));

    reducer.apply_state_update(&VoiceStateUpdateEvent {
        channel_id: c1.to_string(),
        user_id: beto.to_string(),
        muted: true,
        camera_on: false,
        screen_sharing: false,
    });
    assert_eq!(reducer.room_members(c1).len(), 1);

    let joined = reducer
        .apply_voice_joined(
            c1,
            &VoiceJoinedEvent {
                channel_id: c1.to_string(),
                peer_id: "peer-1".into(),
                video_codec: "h264".into(),
                members: vec![
                    VoiceState {
                        user_id: me.to_string(),
                        muted: true,
                        camera_on: false,
                        screen_sharing: false,
                    },
                    VoiceState {
                        user_id: ana.to_string(),
                        muted: false,
                        camera_on: true,
                        screen_sharing: false,
                    },
                ],
                active_speakers: vec![ana.to_string()],
            },
        )
        .unwrap();
    assert_eq!(joined.video_codec, VideoCodec::H264);

    // Leave in another channel does not clear speakers in c1.
    let active = ActivePeerRef {
        channel: c1,
        peer_id: "peer-1",
    };
    let other = reducer.apply_leave(
        me,
        Some(active),
        &VoiceLeaveEvent {
            channel_id: c2.to_string(),
            user_id: ana.to_string(),
            peer_id: "peer-other".into(),
        },
    );
    assert!(!other.ended_active_call);
    assert_eq!(
        reducer
            .room_members(c1)
            .into_iter()
            .find(|(u, _, _)| *u == ana)
            .unwrap()
            .2,
        SpeakingState::Selected
    );

    // Self-leave with matching peer_id ends the call while preserving other occupants.
    let self_leave = reducer.apply_leave(
        me,
        Some(active),
        &VoiceLeaveEvent {
            channel_id: c1.to_string(),
            user_id: me.to_string(),
            peer_id: "peer-1".into(),
        },
    );
    assert!(self_leave.ended_active_call);
    let remaining = reducer.room_members(c1);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, ana);
    assert!(!reducer.audio_routes_known());
}

#[test]
fn speaker_sets_cover_missing_empty_two_selected_mute_and_stale_after_rejoin() {
    let (me, ana, beto) = (id(10), id(11), id(12));
    let c1 = id(30);
    let mut reducer = VoiceReducer::new(Generation::for_test(1));

    // Missing speaker set (before voice_joined / active_speaker_update) -> Unknown.
    reducer.apply_state_update(&VoiceStateUpdateEvent {
        channel_id: c1.to_string(),
        user_id: ana.to_string(),
        muted: false,
        camera_on: false,
        screen_sharing: false,
    });
    assert_eq!(reducer.room_members(c1)[0].2, SpeakingState::Unknown);

    // Empty speaker set -> NotSelected.
    reducer
        .apply_voice_joined(
            c1,
            &VoiceJoinedEvent {
                channel_id: c1.to_string(),
                peer_id: "p1".into(),
                video_codec: "vp8".into(),
                members: vec![
                    VoiceState {
                        user_id: me.to_string(),
                        muted: false,
                        camera_on: false,
                        screen_sharing: false,
                    },
                    VoiceState {
                        user_id: ana.to_string(),
                        muted: false,
                        camera_on: false,
                        screen_sharing: false,
                    },
                    VoiceState {
                        user_id: beto.to_string(),
                        muted: false,
                        camera_on: false,
                        screen_sharing: false,
                    },
                ],
                active_speakers: Vec::new(),
            },
        )
        .unwrap();
    for (_, _, speaking) in reducer.room_members(c1) {
        assert_eq!(speaking, SpeakingState::NotSelected);
    }

    // Two selected (including self).
    reducer.apply_active_speakers(&ActiveSpeakerUpdateEvent {
        channel_id: c1.to_string(),
        user_ids: vec![me.to_string(), ana.to_string()],
    });
    let members = reducer.room_members(c1);
    assert_eq!(
        members.iter().find(|(u, _, _)| *u == me).unwrap().2,
        SpeakingState::Selected
    );
    assert_eq!(
        members.iter().find(|(u, _, _)| *u == ana).unwrap().2,
        SpeakingState::Selected
    );
    assert_eq!(
        members.iter().find(|(u, _, _)| *u == beto).unwrap().2,
        SpeakingState::NotSelected
    );

    // Remote mute clears speaking immediately even before active_speaker_update.
    reducer.apply_state_update(&VoiceStateUpdateEvent {
        channel_id: c1.to_string(),
        user_id: ana.to_string(),
        muted: true,
        camera_on: false,
        screen_sharing: false,
    });
    assert_eq!(
        reducer
            .room_members(c1)
            .iter()
            .find(|(u, _, _)| *u == ana)
            .unwrap()
            .2,
        SpeakingState::NotSelected
    );

    // Clearing room speakers on rejoin prevents stale speaker state.
    reducer.clear_room_speakers(c1);
    assert_eq!(
        reducer
            .room_members(c1)
            .iter()
            .find(|(u, _, _)| *u == me)
            .unwrap()
            .2,
        SpeakingState::Unknown
    );
}

fn joined_route_reducer(c1: Uuid, me: Uuid, ana: Uuid, beto: Uuid) -> VoiceReducer {
    let mut reducer = VoiceReducer::new(Generation::for_test(1));
    reducer
        .apply_voice_joined(
            c1,
            &VoiceJoinedEvent {
                channel_id: c1.to_string(),
                peer_id: "peer-cur".into(),
                video_codec: "h264".into(),
                members: vec![
                    VoiceState {
                        user_id: me.to_string(),
                        muted: true,
                        camera_on: false,
                        screen_sharing: false,
                    },
                    VoiceState {
                        user_id: ana.to_string(),
                        muted: false,
                        camera_on: true,
                        screen_sharing: false,
                    },
                    VoiceState {
                        user_id: beto.to_string(),
                        muted: false,
                        camera_on: true,
                        screen_sharing: true,
                    },
                ],
                active_speakers: Vec::new(),
            },
        )
        .unwrap();
    reducer
}

#[test]
fn audio_and_video_routes_compare_revision_per_type_and_enforce_peer_id() {
    let (me, ana, beto, ghost) = (id(10), id(11), id(12), id(13));
    let c1 = id(30);
    let mut reducer = joined_route_reducer(c1, me, ana, beto);
    let active = ActivePeerRef {
        channel: c1,
        peer_id: "peer-cur",
    };

    // Audio route revision 5 applied.
    assert!(
        reducer
            .apply_audio_routes(
                Some(active),
                &VoiceAudioRoutesEvent {
                    channel_id: c1.to_string(),
                    peer_id: "peer-cur".into(),
                    revision: 5,
                    routes: vec![VoiceAudioRoute {
                        track_id: "papo-audio-0".into(),
                        user_id: ana.to_string(),
                    }],
                },
            )
            .is_some()
    );

    // Video route with lower revision (2 < 5) IS valid because revision is compared per event type.
    assert_eq!(
        reducer.apply_video_routes(
            Some(active),
            &VoiceVideoRoutesEvent {
                channel_id: c1.to_string(),
                peer_id: "peer-cur".into(),
                revision: 2,
                routes: vec![
                    VoiceVideoRoute {
                        track_id: "papo-video-0".into(),
                        user_id: ana.to_string(),
                        kind: "video".into(),
                        first_seq: None,
                        rtp_route_id: Some(1),
                        rtp_route_ext_id: Some(14),
                    },
                    // Ghost is not in room -> dropped.
                    VoiceVideoRoute {
                        track_id: "papo-video-1".into(),
                        user_id: ghost.to_string(),
                        kind: "video".into(),
                        first_seq: None,
                        rtp_route_id: Some(1),
                        rtp_route_ext_id: Some(14),
                    },
                ],
            },
        ),
        Some(2)
    );
    assert_eq!(reducer.video_routes().len(), 1);
    assert_eq!(reducer.video_routes()["papo-video-0"].participant, ana);
    assert_eq!(
        reducer.video_routes()["papo-video-0"].kind,
        VideoKind::Camera
    );

    // Older video revision (1 < 2) and wrong peer_id are dropped.
    assert!(
        reducer
            .apply_video_routes(
                Some(active),
                &VoiceVideoRoutesEvent {
                    channel_id: c1.to_string(),
                    peer_id: "peer-cur".into(),
                    revision: 1,
                    routes: Vec::new(),
                },
            )
            .is_none()
    );
    assert!(
        reducer
            .apply_video_routes(
                Some(active),
                &VoiceVideoRoutesEvent {
                    channel_id: c1.to_string(),
                    peer_id: "peer-old".into(),
                    revision: 99,
                    routes: Vec::new(),
                },
            )
            .is_none()
    );
}

#[test]
fn video_routes_handle_reassignment_departed_member_and_release() {
    let (me, ana, beto) = (id(10), id(11), id(12));
    let c1 = id(30);
    let mut reducer = joined_route_reducer(c1, me, ana, beto);
    let active = ActivePeerRef {
        channel: c1,
        peer_id: "peer-cur",
    };

    // Reassignment of papo-video-0 from ana to beto with first_seq.
    assert_eq!(
        reducer.apply_video_routes(
            Some(active),
            &VoiceVideoRoutesEvent {
                channel_id: c1.to_string(),
                peer_id: "peer-cur".into(),
                revision: 3,
                routes: vec![VoiceVideoRoute {
                    track_id: "papo-video-0".into(),
                    user_id: beto.to_string(),
                    kind: "screen".into(),
                    first_seq: Some(4200),
                    rtp_route_id: Some(1),
                    rtp_route_ext_id: Some(14),
                }],
            },
        ),
        Some(3)
    );
    assert_eq!(reducer.video_routes()["papo-video-0"].participant, beto);
    assert_eq!(
        reducer.video_routes()["papo-video-0"].kind,
        VideoKind::Screen
    );
    assert_eq!(reducer.video_routes()["papo-video-0"].first_seq, Some(4200));

    // When beto leaves, their video route is dropped immediately.
    let leave = reducer.apply_leave(
        me,
        Some(active),
        &VoiceLeaveEvent {
            channel_id: c1.to_string(),
            user_id: beto.to_string(),
            peer_id: "peer-beto".into(),
        },
    );
    assert!(!leave.ended_active_call);
    assert!(leave.routes.video_changed);
    assert!(reducer.video_routes().is_empty());

    // Explicit release snapshot (revision 4, empty routes) also leaves empty routes.
    assert_eq!(
        reducer.apply_video_routes(
            Some(active),
            &VoiceVideoRoutesEvent {
                channel_id: c1.to_string(),
                peer_id: "peer-cur".into(),
                revision: 4,
                routes: Vec::new(),
            },
        ),
        Some(4)
    );
    assert!(reducer.video_routes().is_empty());
}
