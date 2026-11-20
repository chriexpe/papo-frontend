//! Observable call and voice-room projection joining admitted names, avatars and `connect_voice`.
//! Uses only `papo-sdk`'s public API; platform widgets, surfaces and copy belong to clients.

use futures_util::{Stream, StreamExt, stream};
use papo_sdk::{
    AccountId, CallAttemptId, CallError, CallHandle, CallPhase, CallSettings, CallSnapshot,
    CallWarning, CaptureState, ConnectionState, ConnectionStatus, Generation, MediaTransportState,
    PlaybackSettings, ResourceSnapshot, RoomsSnapshot, Session, SpeakingState, Uuid, VideoCodec,
    VideoDestinationId, VideoKind, VoiceDeviceState,
    permissions::{VoiceAccess, voice_access},
};
use std::{collections::BTreeMap, pin::Pin, sync::Arc};

/// Projected voice-connect capability for one channel, independent of `read_channel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceCapability {
    /// Admitted evidence confirms a voice channel with `connect_voice` while online.
    Allowed,
    /// Admitted evidence confirms `connect_voice` is denied or the channel was deleted.
    Denied,
    /// Target channel is not a voice channel.
    NotVoiceChannel,
    /// Offline, pre-reconcile, or missing/uncertain permission evidence.
    Unknown,
}

/// Stable identity key for one participant inside a specific call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallParticipantKey {
    /// Owning account.
    pub account: AccountId,
    /// Call attempt identity.
    pub attempt: CallAttemptId,
    /// Backend user identity.
    pub user: Uuid,
}

/// One active video route attached to its publishing participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantVideoBinding {
    /// SFU track identifier (`papo-video-N`).
    pub track_id: String,
    /// Camera or screen share.
    pub kind: VideoKind,
    /// First RTP sequence number for this occupant when the slot was reused.
    pub first_seq: Option<u16>,
    /// Currently attached host video destination identity, if bound.
    pub destination_id: Option<VideoDestinationId>,
}

/// View-ready participant inside an active call, carrying admitted name/avatar and attached video.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallParticipantItem {
    /// Stable `(account, attempt, user)` key.
    pub key: CallParticipantKey,
    /// Backend user identity.
    pub user: Uuid,
    /// Admitted username from `ResourceSnapshot`.
    pub username: Option<String>,
    /// Admitted nickname from `ResourceSnapshot`.
    pub nickname: Option<String>,
    /// Admitted profile avatar format, if the profile was loaded.
    pub avatar_format: Option<String>,
    /// Known server-reported mute/camera/screen state (`None` only when cold).
    pub device: Option<VoiceDeviceState>,
    /// Server-driven active-speaker state; unaffected by local playback gain or mute.
    pub speaking: SpeakingState,
    /// Independent local receiver playback gain (`0..=200`) and mute.
    pub playback: PlaybackSettings,
    /// Video tracks belonging to this participant; video always stays attached to its participant.
    pub video_tracks: Vec<ParticipantVideoBinding>,
}

/// View-ready occupant of an observed voice room (watched with or without an active call).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomParticipantItem {
    /// Backend user identity.
    pub user: Uuid,
    /// Admitted username from `ResourceSnapshot`.
    pub username: Option<String>,
    /// Admitted nickname from `ResourceSnapshot`.
    pub nickname: Option<String>,
    /// Admitted profile avatar format, if the profile was loaded.
    pub avatar_format: Option<String>,
    /// Device state; `None` for cold presence (`presence_sync.user_voice`).
    pub device: Option<VoiceDeviceState>,
    /// Server-driven active-speaker state (`Unknown` before speaker data arrives).
    pub speaking: SpeakingState,
}

/// View-ready occupancy of one admitted voice channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceRoomView {
    /// Voice channel identity.
    pub channel: Uuid,
    /// Occupants sorted by user UUID with admitted names and avatars.
    pub participants: Vec<RoomParticipantItem>,
}

/// View-ready projection of the session's active call attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCallView {
    /// Monotonic publication version for this attempt.
    pub version: u64,
    /// Account owning this attempt.
    pub account: AccountId,
    /// Connection generation owning this attempt.
    pub generation: Generation,
    /// Unique attempt identifier.
    pub attempt: CallAttemptId,
    /// Active voice channel identity.
    pub channel: Uuid,
    /// Active call phase (`Preparing` → `Joining` → `Joined` → `Connected`); terminal `Ended`
    /// transitions are observed on `CallHandle::subscribe()` when `CallView::active` clears to `None`.
    pub phase: CallPhase,
    /// Room video codec announced by `voice_joined`.
    pub video_codec: Option<VideoCodec>,
    /// Desired local mute, camera and facing settings.
    pub settings: CallSettings,
    /// Actual local microphone capture state.
    pub mic_capture: CaptureState,
    /// Actual local camera capture state.
    pub camera_capture: CaptureState,
    /// WebRTC transport state.
    pub transport: MediaTransportState,
    /// Non-fatal warnings.
    pub warnings: Vec<CallWarning>,
    /// Joined participants with admitted names, avatars, speaking state and attached video bindings.
    pub participants: Vec<CallParticipantItem>,
    /// Whether `voice_audio_routes` has been received for this peer.
    pub audio_routes_known: bool,
    /// Informative host audio focus state.
    pub host_audio_focus: bool,
}

/// Atomic view of one account's active call and admitted voice rooms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallView {
    /// Connection generation owning this projection.
    pub generation: Generation,
    /// Active call projection, if any.
    pub active: Option<ActiveCallView>,
    /// Non-empty visible voice rooms filtered by admitted `connect_voice`.
    pub rooms: BTreeMap<Uuid, VoiceRoomView>,
}

/// Atomic baseline snapshot plus coalesced update stream closing on session revocation.
pub struct CallSubscription {
    /// Current view captured after subscribing to all source watches.
    pub snapshot: CallView,
    /// Subsequent views on call, room, resource or connection updates.
    pub stream: Pin<Box<dyn Stream<Item = CallView> + Send>>,
}

/// Observable call control and voice-room service bound to one account session.
#[derive(Clone)]
pub struct CallService {
    session: Session,
}

impl CallService {
    /// Binds to `session` without starting network requests or loading profiles.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Joins `channel` on this session's socket, returning an attempt-scoped `CallHandle`.
    ///
    /// # Errors
    /// Returns typed precondition or lease failures from `Calls::join`.
    pub fn join(&self, channel: Uuid) -> Result<CallHandle, CallError> {
        self.session.calls().join(channel)
    }

    /// Reattaches to this session's active call handle after navigation or view rebuild.
    #[must_use]
    pub fn active(&self) -> Option<CallHandle> {
        self.session.calls().active()
    }

    /// Projects `connect_voice` for `channel` from the latest online admitted resource snapshot.
    ///
    /// Independent of `read_channel`; offline or incomplete evidence returns `Unknown`.
    #[must_use]
    pub fn connect_voice(&self, channel: Uuid) -> VoiceCapability {
        let status = *self.session.subscribe_connection().borrow();
        let resources = Arc::clone(&self.session.subscribe_resources().borrow());
        if self.session.is_closed()
            || status.state != ConnectionState::Online
            || status.generation != self.session.generation()
            || resources.generation != status.generation
        {
            return VoiceCapability::Unknown;
        }
        match voice_access(&resources, self.session.account().user(), channel) {
            VoiceAccess::Allowed => VoiceCapability::Allowed,
            VoiceAccess::Denied => VoiceCapability::Denied,
            VoiceAccess::NotVoiceChannel => VoiceCapability::NotVoiceChannel,
            VoiceAccess::Unknown | _ => VoiceCapability::Unknown,
        }
    }

    /// Subscribes atomically to rooms and active call state joined with admitted user/profile data.
    #[must_use]
    pub fn subscribe(&self) -> CallSubscription {
        let calls = self.session.calls();
        let mut rooms_rx = calls.subscribe_rooms();
        let mut active_rx = calls.subscribe_active();
        let mut resources_rx = self.session.subscribe_resources();
        let mut connection_rx = self.session.subscribe_connection();

        let rooms = rooms_rx.borrow_and_update().clone();
        let active = active_rx.borrow_and_update().clone();
        let resources = Arc::clone(&resources_rx.borrow_and_update());
        let status = *connection_rx.borrow_and_update();
        let snapshot = project(&self.session, &rooms, active.as_ref(), &resources, status);

        let session = self.session.clone();
        let initial_last = snapshot.clone();
        let changes = stream::unfold(
            (
                rooms_rx,
                active_rx,
                resources_rx,
                connection_rx,
                session,
                initial_last,
            ),
            move |(
                mut rooms_rx,
                mut active_rx,
                mut resources_rx,
                mut connection_rx,
                session,
                mut last_view,
            )| async move {
                loop {
                    tokio::select! {
                        result = rooms_rx.changed() => result.ok()?,
                        result = active_rx.changed() => result.ok()?,
                        result = resources_rx.changed() => result.ok()?,
                        result = connection_rx.changed() => result.ok()?,
                    }
                    if session.is_closed() {
                        return None;
                    }
                    let rooms = rooms_rx.borrow_and_update().clone();
                    let active = active_rx.borrow_and_update().clone();
                    let resources = Arc::clone(&resources_rx.borrow_and_update());
                    let status = *connection_rx.borrow_and_update();
                    let view = project(&session, &rooms, active.as_ref(), &resources, status);
                    if view == last_view {
                        continue;
                    }
                    last_view = view.clone();
                    return Some((
                        view,
                        (
                            rooms_rx,
                            active_rx,
                            resources_rx,
                            connection_rx,
                            session,
                            last_view,
                        ),
                    ));
                }
            },
        );
        CallSubscription {
            snapshot,
            stream: Box::pin(changes.fuse()),
        }
    }
}

fn project(
    session: &Session,
    rooms: &RoomsSnapshot,
    active: Option<&CallSnapshot>,
    resources: &ResourceSnapshot,
    status: ConnectionStatus,
) -> CallView {
    let mut view = CallView {
        generation: status.generation,
        active: None,
        rooms: BTreeMap::new(),
    };
    if session.is_closed()
        || status.state != ConnectionState::Online
        || session.generation() != status.generation
    {
        return view;
    }
    let matching_resources = resources.generation == status.generation;
    if rooms.generation == status.generation {
        for (&channel, room) in &rooms.rooms {
            let participants = room
                .members
                .iter()
                .map(|member| {
                    let (username, nickname, avatar_format) =
                        user_identity(matching_resources, resources, member.user);
                    RoomParticipantItem {
                        user: member.user,
                        username,
                        nickname,
                        avatar_format,
                        device: member.device,
                        speaking: room.speaking_state(member.user, member.device),
                    }
                })
                .collect();
            view.rooms.insert(
                channel,
                VoiceRoomView {
                    channel,
                    participants,
                },
            );
        }
    }
    if let Some(call) = active
        && call.generation == status.generation
    {
        let participants = call
            .participants
            .iter()
            .map(|participant| {
                let (username, nickname, avatar_format) =
                    user_identity(matching_resources, resources, participant.user);
                let video_tracks = call
                    .video_bindings
                    .iter()
                    .filter(|binding| binding.participant == participant.user)
                    .map(|binding| ParticipantVideoBinding {
                        track_id: binding.track_id.clone(),
                        kind: binding.kind,
                        first_seq: binding.first_seq,
                        destination_id: binding.destination_id,
                    })
                    .collect();
                CallParticipantItem {
                    key: CallParticipantKey {
                        account: participant.account,
                        attempt: participant.attempt,
                        user: participant.user,
                    },
                    user: participant.user,
                    username,
                    nickname,
                    avatar_format,
                    device: participant.device,
                    speaking: participant.speaking,
                    playback: participant.playback,
                    video_tracks,
                }
            })
            .collect();
        view.active = Some(ActiveCallView {
            version: call.version,
            account: call.account,
            generation: call.generation,
            attempt: call.attempt,
            channel: call.channel,
            phase: call.phase,
            video_codec: call.video_codec,
            settings: call.settings,
            mic_capture: call.mic_capture,
            camera_capture: call.camera_capture,
            transport: call.transport,
            warnings: call.warnings.clone(),
            participants,
            audio_routes_known: call.audio_routes_known,
            host_audio_focus: call.host_audio_focus,
        });
    }
    view
}

fn user_identity(
    matching_resources: bool,
    resources: &ResourceSnapshot,
    user: Uuid,
) -> (Option<String>, Option<String>, Option<String>) {
    if !matching_resources {
        return (None, None, None);
    }
    let summary = resources.users.get(user);
    let profile = resources.profiles.get(user);
    (
        summary.and_then(|u| u.username.clone()),
        summary.and_then(|u| u.nickname.clone()),
        profile.and_then(|p| p.avatar_format.clone()),
    )
}
