//! Deterministic call coordinator tests using fake clock, fake writer FIFO and fake media engine.

use super::{
    coordinator::SessionCalls,
    fake_media::{FakeMediaFactory, FakeStartBehavior, FakeVideoDestination},
    fifo::{QueuedVoiceCommand, VoiceCommandReceiver},
    handle::CallHandle,
    lease::ClientCallLease,
    media_port::{
        CaptureState, IceServerConfig, MediaCommand, MediaEngineFactory, MediaLeaseError,
        MediaReportKind, MediaTransportState, MediaWarning, VideoDestination, VideoDestinationId,
    },
    model::{
        CallConfig, CallEndReason, CallError, CallPhase, CallWarning, SpeakingState, VideoCodec,
        VideoKind, VideoTarget,
    },
    startup::{IceFetchFuture, IceFetcher},
};
use crate::{
    AccountId, ChannelKind, ChannelRecord, ClientError, ConnectionState, Generation, Overwrite,
    OverwriteKey, ResourceSnapshot, ResourceTable, RoleFlags, RoleRecord, ServerAddress,
    ServerRecord, UserRecord, Uuid,
};
use papo_api::gateway::{
    Command, ErrorEvent, Event, KnownEvent, VoiceAnswerEvent, VoiceAudioRoute,
    VoiceAudioRoutesEvent, VoiceICECandidateEvent, VoiceJoinedEvent, VoiceLeaveEvent, VoiceState,
    VoiceVideoRoute, VoiceVideoRoutesEvent,
};
use std::{
    any::Any,
    collections::VecDeque,
    sync::{Arc, Barrier, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn account(n: u128) -> AccountId {
    let server = ServerAddress::parse("https://voice.example").unwrap().id();
    AccountId::new(server, id(n))
}

enum FakeIceStep {
    Ok(Vec<IceServerConfig>),
    Gated(oneshot::Receiver<Result<Vec<IceServerConfig>, ClientError>>),
}

struct FakeIceFetcher {
    steps: Mutex<VecDeque<FakeIceStep>>,
}

impl FakeIceFetcher {
    fn immediate() -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(VecDeque::new()),
        })
    }

    fn with_gate() -> (
        Arc<Self>,
        oneshot::Sender<Result<Vec<IceServerConfig>, ClientError>>,
    ) {
        let (tx, rx) = oneshot::channel();
        let fetcher = Arc::new(Self {
            steps: Mutex::new(VecDeque::from([FakeIceStep::Gated(rx)])),
        });
        (fetcher, tx)
    }
}

impl IceFetcher for FakeIceFetcher {
    fn fetch(&self, _generation: Generation) -> IceFetchFuture {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| FakeIceStep::Ok(Vec::new()));
        Box::pin(async move {
            match step {
                FakeIceStep::Ok(servers) => Ok(servers),
                FakeIceStep::Gated(rx) => rx.await.unwrap_or(Err(ClientError::Closed)),
            }
        })
    }
}

fn voice_resources(
    generation: Generation,
    user: Uuid,
    role: Uuid,
    channels: Vec<(Uuid, ChannelKind, bool, bool)>,
) -> Arc<ResourceSnapshot> {
    let mut snapshot = ResourceSnapshot::empty(generation);
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
            status_message: None,
            created_at: None,
            roles: Some(vec![role]),
        },
    );
    snapshot.members.replace([user].into_iter());
    snapshot.channels =
        ResourceTable::from_rows(channels.into_iter().map(|(ch_id, kind, read, voice)| {
            (
                ch_id,
                ChannelRecord {
                    id: ch_id,
                    name: Some("v".into()),
                    kind,
                    position: Some(1),
                    topic: None,
                    parent_id: None,
                    permissions: vec![Overwrite {
                        key: OverwriteKey::Canonical(role),
                        read_channel: read,
                        send_messages: Some(false),
                        delete_messages: Some(false),
                        connect_voice: Some(voice),
                    }],
                    created_at: None,
                    last_message: None,
                    last_read_message: None,
                    last_read_at: None,
                    notification_settings: None,
                },
            )
        }));
    Arc::new(snapshot)
}

struct Harness {
    calls: Arc<SessionCalls>,
    fifo: VoiceCommandReceiver,
    media: FakeMediaFactory,
    root: CancellationToken,
    gen_id: Generation,
}

impl Harness {
    fn new(user: Uuid, role: Uuid, channels: Vec<(Uuid, ChannelKind, bool, bool)>) -> Self {
        Self::with_config(user, role, channels, CallConfig::default())
    }

    fn with_config(
        user: Uuid,
        role: Uuid,
        channels: Vec<(Uuid, ChannelKind, bool, bool)>,
        config: CallConfig,
    ) -> Self {
        let gen_id = Generation::for_test(1);
        let lease = Arc::new(ClientCallLease::default());
        let calls = SessionCalls::new(account(user.as_u128()), gen_id, config, lease);
        let fifo = calls.attach_socket(gen_id);
        let resources = voice_resources(gen_id, user, role, channels);
        calls.on_connection_status(
            gen_id,
            ConnectionState::Online,
            &resources,
            Some(&tokio::runtime::Handle::current()),
        );
        Self {
            calls,
            fifo,
            media: FakeMediaFactory::new(),
            root: CancellationToken::new(),
            gen_id,
        }
    }

    fn join_with(&self, channel: Uuid, ice: Arc<dyn IceFetcher>) -> Result<CallHandle, CallError> {
        let factory: Arc<dyn MediaEngineFactory> = Arc::new(self.media.clone());
        let (attempt, cell) = self.calls.join(
            channel,
            Some(factory),
            ice,
            &tokio::runtime::Handle::current(),
            &self.root,
        )?;
        Ok(CallHandle::from_parts(
            self.calls.account,
            attempt,
            Arc::clone(&self.calls),
            cell,
            tokio::runtime::Handle::current(),
        ))
    }

    fn deliver(&self, event: KnownEvent) {
        self.calls.on_gateway_event(
            self.gen_id,
            &Event::Known(event),
            &tokio::runtime::Handle::current(),
        );
    }

    async fn next_sent_command(&mut self) -> Command {
        loop {
            let item: QueuedVoiceCommand = self.fifo.recv().await.expect("voice fifo closed");
            if self.calls.should_send(&item, self.gen_id) {
                return item.command;
            }
        }
    }

    fn drain_sent_commands(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        while let Some(item) = self.fifo.try_recv() {
            if self.calls.should_send(&item, self.gen_id) {
                out.push(item.command);
            }
        }
        out
    }
}

#[tokio::test(start_paused = true)]
async fn cancel_during_ice_fetch_and_late_success_sends_nothing() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);
    let (ice, gate) = FakeIceFetcher::with_gate();

    let handle = h.join_with(c1, ice).unwrap();
    assert_eq!(handle.snapshot().phase, CallPhase::Preparing);
    handle.leave();
    assert_eq!(
        handle.snapshot().phase,
        CallPhase::Ended(CallEndReason::LocalLeave)
    );

    // Late ICE success after cancel must not start media or emit voice_join/voice_leave.
    let _ = gate.send(Ok(Vec::new()));
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(h.media.start_count(), 0);
    assert!(h.drain_sent_commands().is_empty());
}

#[tokio::test(start_paused = true)]
async fn cancel_during_media_start_and_late_success_stops_lease_and_sends_nothing() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);
    let (tx, rx) = oneshot::channel();
    h.media.push_start(FakeStartBehavior::Gated(rx));
    h.media.set_auto_drain(false);

    let handle = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    h.media.wait_starts(1).await;
    handle.leave();
    assert_eq!(
        handle.snapshot().phase,
        CallPhase::Ended(CallEndReason::LocalLeave)
    );

    // Late media start completion after cancel is immediately stopped and never sends voice_join.
    let _ = tx.send(Ok(()));
    let lease = h.media.wait_lease(1).await;
    lease.wait_stopped().await;
    assert!(lease.commands().is_empty());
    assert!(h.drain_sent_commands().is_empty());

    h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    tokio::task::yield_now().await;
    assert_eq!(h.media.start_count(), 1);
    lease.emit(MediaReportKind::Drained);
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    assert_eq!(h.media.start_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn full_media_queue_ends_call_and_stops_lease() {
    assert_media_command_failure(MediaLeaseError::Full).await;
}

#[tokio::test(start_paused = true)]
async fn closed_media_queue_ends_call_and_stops_lease() {
    assert_media_command_failure(MediaLeaseError::Closed).await;
}

async fn assert_media_command_failure(error: MediaLeaseError) {
    let (me, role, c1) = (id(10), id(20), id(30));
    let mut h = Harness::with_config(
        me,
        role,
        vec![(c1, ChannelKind::Voice, false, true)],
        CallConfig {
            command_queue_capacity: 1,
            ..CallConfig::default()
        },
    );
    h.media.set_auto_drain(false);
    let call = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let lease = h.media.wait_lease(1).await;
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
        video_codec: "vp8".into(),
        members: Vec::new(),
        active_speakers: Vec::new(),
    })));
    let delivered = lease.commands().len();
    lease.set_command_error(Some(error));

    call.set_muted(false).unwrap();

    assert_eq!(
        call.snapshot().phase,
        CallPhase::Ended(CallEndReason::MediaCommandFailed(error))
    );
    assert_eq!(lease.commands().len(), delivered);
    assert!(lease.is_stopped());
    assert!(matches!(
        h.drain_sent_commands().as_slice(),
        [Command::VoiceLeave { .. }]
    ));

    let replacement = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    lease.emit_for(replacement.attempt(), MediaReportKind::Drained);
    tokio::task::yield_now().await;
    assert_eq!(h.media.start_count(), 1);
    lease.emit(MediaReportKind::Drained);
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    assert_eq!(h.media.start_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn superseded_subscription_wakeup_preserves_current_deadline() {
    let (me, role, channel) = (id(10), id(20), id(30));
    let h = Harness::new(me, role, vec![(channel, ChannelKind::Voice, false, true)]);
    let call = h.join_with(channel, FakeIceFetcher::immediate()).unwrap();
    let now = tokio::time::Instant::now();
    let current_at = now + Duration::from_secs(1);
    let superseded_at = now + Duration::from_secs(2);
    h.calls
        .state
        .lock()
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .scheduled_wakeup = Some(current_at);

    h.calls.on_subscription_wakeup(
        call.attempt(),
        superseded_at,
        &tokio::runtime::Handle::current(),
    );

    assert_eq!(
        h.calls
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .scheduled_wakeup,
        Some(current_at)
    );
}

#[tokio::test(start_paused = true)]
async fn shutdown_rejects_replacement_while_holding_the_device_lease() {
    let (me, role, channel) = (id(10), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(channel, ChannelKind::Voice, false, true)]);
    h.media.set_auto_drain(false);
    let old = h.join_with(channel, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let lease = h.media.wait_lease(1).await;
    let calls = Arc::clone(&h.calls);
    let shutdown = tokio::spawn(async move { calls.stop_and_drain(CallEndReason::Shutdown).await });
    lease.wait_stopped().await;

    assert!(matches!(
        h.join_with(channel, FakeIceFetcher::immediate()),
        Err(CallError::Closed)
    ));
    assert_eq!(
        old.snapshot().phase,
        CallPhase::Ended(CallEndReason::Shutdown)
    );
    assert!(!shutdown.is_finished());
    lease.emit(MediaReportKind::Drained);
    shutdown.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn queued_leave_is_discarded_when_its_socket_generation_ends() {
    let (me, role, channel) = (id(10), id(20), id(30));
    let channels = vec![(channel, ChannelKind::Voice, false, true)];
    let mut h = Harness::new(me, role, channels.clone());
    let call = h.join_with(channel, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    call.leave();
    let leave = h.fifo.try_recv().unwrap();
    assert!(matches!(leave.command, Command::VoiceLeave { .. }));

    let generation = Generation::for_test(2);
    h.calls.on_connection_status(
        generation,
        ConnectionState::Online,
        &voice_resources(generation, me, role, channels),
        None,
    );

    assert!(!h.calls.should_send(&leave, h.gen_id));
}

#[tokio::test(start_paused = true)]
async fn replacement_join_can_queue_behind_leave_with_one_ordinary_slot() {
    let (me, role, channel) = (id(10), id(20), id(30));
    let mut h = Harness::with_config(
        me,
        role,
        vec![(channel, ChannelKind::Voice, false, true)],
        CallConfig {
            command_queue_capacity: 1,
            ..CallConfig::default()
        },
    );
    h.join_with(channel, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));

    let replacement = h.join_with(channel, FakeIceFetcher::immediate()).unwrap();
    h.media.wait_lease(2).await;

    assert_eq!(replacement.snapshot().phase, CallPhase::Joining);
    assert!(matches!(
        h.drain_sent_commands().as_slice(),
        [Command::VoiceLeave { .. }, Command::VoiceJoin { .. },]
    ));
}

#[tokio::test(start_paused = true)]
async fn leave_after_voice_join_sends_leave_and_missing_voice_joined_times_out() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let config = CallConfig {
        join_timeout: Duration::from_secs(5),
        ..CallConfig::default()
    };
    let mut h = Harness::with_config(
        me,
        role,
        vec![(c1, ChannelKind::Voice, false, true)],
        config,
    );

    let handle = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    assert_eq!(handle.snapshot().phase, CallPhase::Joining);

    // Missing voice_joined -> JoinTimeout and VoiceLeave sent.
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        handle.snapshot().phase,
        CallPhase::Ended(CallEndReason::JoinTimeout)
    );
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceLeave { .. }
    ));
}

async fn assert_stale_peer_events_dropped(
    h: &Harness,
    call: &CallHandle,
    channel: Uuid,
    remote: Uuid,
    stale_peer_id: &str,
) {
    let lease = h.media.wait_lease(2).await;
    let before_cmds = lease.commands().len();
    h.deliver(KnownEvent::VoiceAnswer(Box::new(VoiceAnswerEvent {
        channel_id: channel.to_string(),
        peer_id: stale_peer_id.into(),
        sdp: "stale".into(),
    })));
    h.deliver(KnownEvent::VoiceICECandidate(Box::new(
        VoiceICECandidateEvent {
            channel_id: channel.to_string(),
            peer_id: stale_peer_id.into(),
            candidate: "candidate:stale".into(),
            sdp_mid: None,
            sdp_mline_index: None,
        },
    )));
    h.deliver(KnownEvent::VoiceAudioRoutes(Box::new(
        VoiceAudioRoutesEvent {
            channel_id: channel.to_string(),
            peer_id: stale_peer_id.into(),
            revision: 1,
            routes: vec![VoiceAudioRoute {
                track_id: "papo-audio-0".into(),
                user_id: remote.to_string(),
            }],
        },
    )));
    h.deliver(KnownEvent::VoiceVideoRoutes(Box::new(
        VoiceVideoRoutesEvent {
            channel_id: channel.to_string(),
            peer_id: stale_peer_id.into(),
            revision: 1,
            routes: vec![VoiceVideoRoute {
                track_id: "papo-video-0".into(),
                user_id: remote.to_string(),
                kind: "video".into(),
                first_seq: None,
                rtp_route_id: Some(1),
                rtp_route_ext_id: Some(14),
            }],
        },
    )));
    h.deliver(KnownEvent::Error(Box::new(ErrorEvent {
        message: "stale".into(),
        code: Some("voice-invalid-sdp".into()),
        channel_id: Some(channel.to_string()),
        peer_id: Some(stale_peer_id.into()),
    })));
    assert_eq!(call.snapshot().phase, CallPhase::Joined);
    assert_eq!(lease.commands().len(), before_cmds);
    assert!(!call.snapshot().audio_routes_known);
    assert_eq!(call.snapshot().video_bindings, Vec::new());
}

#[tokio::test(start_paused = true)]
async fn back_to_back_rejoin_same_and_different_channel_ignores_old_peer_events() {
    let (me, ana, role, c1, c2) = (id(10), id(11), id(20), id(30), id(31));
    let mut h = Harness::new(
        me,
        role,
        vec![
            (c1, ChannelKind::Voice, false, true),
            (c2, ChannelKind::Voice, false, true),
        ],
    );

    // Attempt A on c1.
    let call_a = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "peer-a".into(),
        video_codec: "h264".into(),
        members: vec![VoiceState {
            user_id: me.to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        active_speakers: vec![me.to_string()],
    })));
    assert_eq!(call_a.snapshot().phase, CallPhase::Joined);
    assert!(call_a.snapshot().settings.muted);
    assert!(!call_a.snapshot().settings.camera_on);

    // Back-to-back A -> A on the same socket.
    let call_a2 = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert_eq!(
        call_a.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));

    // A2 receives voice_joined BEFORE A's late self-leave arrives.
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "peer-a2".into(),
        video_codec: "vp8".into(),
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
        active_speakers: Vec::new(),
    })));
    assert_eq!(call_a2.snapshot().phase, CallPhase::Joined);
    assert_eq!(call_a2.snapshot().video_codec, Some(VideoCodec::Vp8));

    // Late self-leave from peer-a arrives AFTER peer-a2 joined -> MUST NOT end peer-a2!
    h.deliver(KnownEvent::VoiceLeave(Box::new(VoiceLeaveEvent {
        channel_id: c1.to_string(),
        user_id: me.to_string(),
        peer_id: "peer-a".into(),
    })));
    assert_eq!(call_a2.snapshot().phase, CallPhase::Joined);
    assert!(call_a2.snapshot().participants.iter().any(|p| p.user == me));

    // Late answer, candidate, audio/video routes and fatal error from peer-a are all dropped.
    assert_stale_peer_events_dropped(&h, &call_a2, c1, ana, "peer-a").await;

    // Foreign leave (ana) never ends our call.
    h.deliver(KnownEvent::VoiceLeave(Box::new(VoiceLeaveEvent {
        channel_id: c1.to_string(),
        user_id: ana.to_string(),
        peer_id: "peer-ana".into(),
    })));
    assert_eq!(call_a2.snapshot().phase, CallPhase::Joined);

    // Back-to-back A2 -> B (c1 -> c2), with A2's self-leave arriving BEFORE B's voice_joined.
    assert_a2_to_b_with_early_leave(&mut h, me, c1, c2).await;
}

async fn assert_a2_to_b_with_early_leave(h: &mut Harness, me: Uuid, c1: Uuid, c2: Uuid) {
    let call_b = h.join_with(c2, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceLeave(Box::new(VoiceLeaveEvent {
        channel_id: c1.to_string(),
        user_id: me.to_string(),
        peer_id: "peer-a2".into(),
    })));
    assert_eq!(call_b.snapshot().phase, CallPhase::Joining);
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c2.to_string(),
        peer_id: "peer-b".into(),
        video_codec: "h264".into(),
        members: vec![VoiceState {
            user_id: me.to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        active_speakers: Vec::new(),
    })));
    assert_eq!(call_b.snapshot().phase, CallPhase::Joined);
    assert_eq!(call_b.snapshot().channel, c2);
}

#[tokio::test(start_paused = true)]
async fn voice_permissions_unknown_revoke_regrant_and_missing_factory() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);

    // Missing media factory returns MediaUnavailable while rooms remain observable.
    let missing = h.calls.join(
        c1,
        None,
        FakeIceFetcher::immediate(),
        &tokio::runtime::Handle::current(),
        &h.root,
    );
    assert!(matches!(missing, Err(CallError::MediaUnavailable)));

    // Voice granted WITHOUT read_channel succeeds.
    let call = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
        video_codec: "vp8".into(),
        members: vec![VoiceState {
            user_id: me.to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        active_speakers: Vec::new(),
    })));
    assert_eq!(call.snapshot().phase, CallPhase::Joined);

    // Revoking connect_voice ends the active call (sending VoiceLeave), and regrant never revives it.
    let denied = voice_resources(
        h.gen_id,
        me,
        role,
        vec![(c1, ChannelKind::Voice, true, false)],
    );
    h.calls
        .on_resources_updated(&denied, Some(&tokio::runtime::Handle::current()));
    assert_eq!(
        call.snapshot().phase,
        CallPhase::Ended(CallEndReason::Forbidden)
    );
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceLeave { .. }
    ));

    let regranted = voice_resources(
        h.gen_id,
        me,
        role,
        vec![(c1, ChannelKind::Voice, false, true)],
    );
    h.calls
        .on_resources_updated(&regranted, Some(&tokio::runtime::Handle::current()));
    assert_eq!(
        call.snapshot().phase,
        CallPhase::Ended(CallEndReason::Forbidden)
    );
    assert!(h.calls.active_attempt_handle_parts().is_none());

    // Permission becoming Unknown also ends an active call.
    let call2 = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let mut unknown = (*regranted).clone();
    unknown.roles = ResourceTable::default();
    h.calls
        .on_resources_updated(&Arc::new(unknown), Some(&tokio::runtime::Handle::current()));
    assert_eq!(
        call2.snapshot().phase,
        CallPhase::Ended(CallEndReason::PermissionUnknown)
    );
}

fn assert_video_reassignment(
    h: &Harness,
    call: &CallHandle,
    channel: Uuid,
    ana: Uuid,
    beto: Uuid,
) -> (Arc<FakeVideoDestination>, Arc<FakeVideoDestination>) {
    let dest_ana = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(1)));
    let dest_beto = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(2)));
    call.bind_video(ana, VideoKind::Camera, dest_ana.clone())
        .unwrap();
    call.bind_video(beto, VideoKind::Camera, dest_beto.clone())
        .unwrap();

    let lease = h.media.latest_lease().unwrap();
    let apply = |revision, participant: Uuid, rtp_route_id, rtp_route_ext_id| {
        h.deliver(KnownEvent::VoiceVideoRoutes(Box::new(
            VoiceVideoRoutesEvent {
                channel_id: channel.to_string(),
                peer_id: "p1".into(),
                revision,
                routes: vec![VoiceVideoRoute {
                    track_id: "papo-video-0".into(),
                    user_id: participant.to_string(),
                    kind: "video".into(),
                    first_seq: Some(900),
                    rtp_route_id,
                    rtp_route_ext_id,
                }],
            },
        )));
        let snapshot = call.snapshot();
        assert_eq!(snapshot.video_bindings.len(), 1);
        assert_eq!(snapshot.video_bindings[0].participant, participant);
        let (fence, destination_id) = lease
            .commands()
            .iter()
            .rev()
            .find_map(|cmd| {
                if let MediaCommand::SetVideoSlotPlan { slots, .. } = cmd {
                    assert_eq!(slots.len(), 1);
                    let slot = &slots[0];
                    assert_eq!(slot.track_id, "papo-video-0");
                    assert_eq!(slot.participant, participant);
                    Some((slot.fence, slot.destination.as_ref().map(|dest| dest.id())))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(snapshot.video_bindings[0].destination_id, destination_id);
        (fence, destination_id)
    };

    let (old_fence, old_destination) = apply(1, ana, Some(41), Some(14));
    let (new_fence, new_destination) = apply(2, beto, Some(42), Some(14));
    let old_fence = old_fence.unwrap();
    let new_fence = new_fence.unwrap();
    assert_eq!(old_destination, Some(VideoDestinationId::from_raw(1)));
    assert_eq!(new_destination, Some(VideoDestinationId::from_raw(2)));
    assert_eq!(old_fence.extension_id, new_fence.extension_id);
    assert_eq!(new_fence.extension_id, 14);
    assert_eq!(old_fence.assignment_id, 41);
    assert_eq!(new_fence.assignment_id, 42);

    // Even with a legacy sequence hint, incomplete or invalid fences cannot render.
    for (revision, (assignment, extension)) in (3u64..).zip([
        (None, None),
        (Some(42), None),
        (None, Some(14)),
        (Some(0), Some(14)),
        (Some(42), Some(0)),
        (Some(42), Some(15)),
    ]) {
        assert_eq!(apply(revision, beto, assignment, extension), (None, None));
    }
    (dest_ana, dest_beto)
}

#[tokio::test(start_paused = true)]
async fn video_bindings_interest_and_local_playback() {
    let (me, ana, beto, role, c1) = (id(10), id(11), id(12), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);

    let call1 = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
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
                screen_sharing: false,
            },
        ],
        active_speakers: vec![ana.to_string()],
    })));
    let lease = h.media.wait_lease(1).await;

    // Local playback mute & gain do NOT alter speaking state.
    call1.set_playback(ana, 250, true).unwrap();
    let ana_row = call1
        .snapshot()
        .participants
        .iter()
        .find(|p| p.user == ana)
        .copied()
        .unwrap();
    assert_eq!(ana_row.speaking, SpeakingState::Selected);
    assert_eq!(ana_row.playback.gain_percent, 200);
    assert!(ana_row.playback.muted);

    // Separate facts: missing mic warning and camera unavailable do not end the call.
    lease.emit(MediaReportKind::MicCapture(CaptureState::Unavailable));
    lease.emit(MediaReportKind::Warning(
        MediaWarning::MicrophoneUnavailable,
    ));
    lease.emit(MediaReportKind::CameraCapture(CaptureState::Unavailable));
    lease.emit(MediaReportKind::Warning(MediaWarning::CameraUnavailable));
    lease.emit(MediaReportKind::TransportState(
        MediaTransportState::Connected,
    ));
    tokio::task::yield_now().await;
    let snap = call1.snapshot();
    assert_eq!(snap.phase, CallPhase::Connected);
    assert_eq!(snap.mic_capture, CaptureState::Unavailable);
    assert_eq!(snap.camera_capture, CaptureState::Unavailable);
    assert!(snap.warnings.contains(&CallWarning::MicrophoneUnavailable));
    assert!(snap.warnings.contains(&CallWarning::CameraUnavailable));

    // Video interest waits until the first voice_answer arrives.
    call1
        .set_video_interest(vec![VideoTarget {
            participant: ana,
            kind: VideoKind::Camera,
        }])
        .unwrap();
    assert!(h.drain_sent_commands().is_empty());

    h.deliver(KnownEvent::VoiceAnswer(Box::new(VoiceAnswerEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
        sdp: "v=0".into(),
    })));
    assert!(matches!(
        h.next_sent_command().await,
        Command::TrackSubscribe { .. }
    ));

    call1
        .set_video_interest(vec![
            VideoTarget {
                participant: ana,
                kind: VideoKind::Camera,
            },
            VideoTarget {
                participant: beto,
                kind: VideoKind::Camera,
            },
        ])
        .unwrap();

    // Bind destinations for ana and beto, then verify video route reassignment moves slot to beto.
    let (dest_ana, dest_beto) = assert_video_reassignment(&h, &call1, c1, ana, beto);

    // Audio focus loss mutes; regain never unmutes.
    call1.set_muted(false).unwrap();
    assert!(!call1.snapshot().settings.muted);
    call1.on_audio_focus_lost().unwrap();
    assert!(call1.snapshot().settings.muted);
    call1.on_audio_focus_regained().unwrap();
    assert!(call1.snapshot().settings.muted);

    call1.leave();
    assert!(dest_ana.is_revoked());
    assert!(dest_beto.is_revoked());
}

#[tokio::test(start_paused = true)]
async fn stale_handle_is_inert_after_rejoin() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);

    let call1 = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    let call2 = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert_eq!(call1.account(), call2.account());
    assert_ne!(call1.attempt(), call2.attempt());
    assert_eq!(
        call1.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    call2.set_muted(false).unwrap();
    assert_eq!(call1.set_camera(true), Err(CallError::Ended));
    call1.leave();
    assert_eq!(call1.on_audio_focus_lost(), Err(CallError::Ended));
    assert_eq!(call1.on_host_terminated(), Err(CallError::Ended));
    assert!(!call2.snapshot().settings.muted);
    assert!(!call2.snapshot().settings.camera_on);
    assert!(call2.snapshot().phase.is_active());
}

struct PausedRevocation {
    entered: Barrier,
    resume: Barrier,
}

impl PausedRevocation {
    fn new() -> Self {
        Self {
            entered: Barrier::new(2),
            resume: Barrier::new(2),
        }
    }
}

impl VideoDestination for PausedRevocation {
    fn id(&self) -> VideoDestinationId {
        VideoDestinationId::from_raw(1)
    }

    fn revoke(&self) {
        self.entered.wait();
        self.resume.wait();
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}

#[tokio::test(start_paused = true)]
async fn deferred_video_unbind_cannot_send_to_replacement_lease() {
    let (me, remote, role, c1) = (id(10), id(11), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);
    let old = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let destination = Arc::new(PausedRevocation::new());
    old.bind_video(
        remote,
        VideoKind::Camera,
        Arc::clone(&destination) as Arc<dyn VideoDestination>,
    )
    .unwrap();

    let worker = std::thread::spawn(move || old.unbind_video(remote, VideoKind::Camera));
    destination.entered.wait();
    let new = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let lease = h.media.wait_lease(2).await;
    new.set_muted(false).unwrap();
    destination.resume.wait();
    worker.join().unwrap().unwrap();

    assert!(matches!(
        lease.commands().as_slice(),
        [MediaCommand::SetMuted { muted: false }]
    ));
    assert!(
        h.calls
            .active_cell
            .borrow()
            .as_ref()
            .is_some_and(|snap| { snap.attempt == new.attempt() && !snap.settings.muted })
    );
}

#[tokio::test(start_paused = true)]
async fn deferred_permission_teardown_cannot_overwrite_newer_active_or_rooms_snapshot() {
    let (me, remote, role, c1) = (id(10), id(11), id(20), id(30));
    let allowed = vec![(c1, ChannelKind::Voice, false, true)];
    let mut h = Harness::new(me, role, allowed.clone());
    let old = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let destination = Arc::new(PausedRevocation::new());
    old.bind_video(
        remote,
        VideoKind::Camera,
        Arc::clone(&destination) as Arc<dyn VideoDestination>,
    )
    .unwrap();

    let calls = Arc::clone(&h.calls);
    let denied = voice_resources(
        h.gen_id,
        me,
        role,
        vec![(c1, ChannelKind::Voice, false, false)],
    );
    let worker = std::thread::spawn(move || calls.on_resources_updated(&denied, None));
    destination.entered.wait();
    h.calls
        .on_resources_updated(&voice_resources(h.gen_id, me, role, allowed), None);
    let new = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    let rooms_version = h.calls.rooms_cell.borrow().version;
    destination.resume.wait();
    worker.join().unwrap();

    assert_eq!(
        old.snapshot().phase,
        CallPhase::Ended(CallEndReason::Forbidden)
    );
    assert_eq!(h.calls.rooms_cell.borrow().version, rooms_version);
    assert_eq!(
        h.calls
            .active_cell
            .borrow()
            .as_ref()
            .map(|snap| snap.attempt),
        Some(new.attempt())
    );
}

#[tokio::test(start_paused = true)]
async fn deferred_unbind_does_not_reorder_video_commands() {
    let (me, remote, role, c1) = (id(10), id(11), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);
    let call = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    let destination = Arc::new(PausedRevocation::new());
    call.bind_video(
        remote,
        VideoKind::Camera,
        Arc::clone(&destination) as Arc<dyn VideoDestination>,
    )
    .unwrap();

    let old = call.clone();
    let worker = std::thread::spawn(move || old.unbind_video(remote, VideoKind::Camera));
    destination.entered.wait();
    call.bind_video(
        remote,
        VideoKind::Camera,
        Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(2))),
    )
    .unwrap();
    destination.resume.wait();
    worker.join().unwrap().unwrap();

    let lease = h.media.wait_lease(1).await;
    assert!(matches!(
        lease.commands().as_slice(),
        [
            MediaCommand::SetVideoSlotPlan { revision: 1, .. },
            MediaCommand::SetVideoSlotPlan { revision: 2, .. },
            MediaCommand::SetVideoSlotPlan { revision: 3, .. },
        ]
    ));
}

#[tokio::test(start_paused = true)]
async fn camera_intent_is_queued_ahead_of_local_offer_in_fifo() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let mut h = Harness::new(me, role, vec![(c1, ChannelKind::Voice, false, true)]);

    let call = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
        video_codec: "h264".into(),
        members: vec![VoiceState {
            user_id: me.to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        active_speakers: Vec::new(),
    })));
    let lease = h.media.wait_lease(1).await;

    call.set_camera(true).unwrap();
    lease.emit(MediaReportKind::LocalOffer { sdp: "v=0".into() });
    tokio::task::yield_now().await;

    let first = h.next_sent_command().await;
    let second = h.next_sent_command().await;
    assert!(matches!(first, Command::VoiceCamera { on: true, .. }));
    assert!(matches!(second, Command::VoiceOffer { .. }));
}

#[tokio::test(start_paused = true)]
async fn disconnect_ends_call_without_auto_rejoin_and_fifo_overflow_ends_call() {
    let (me, role, c1) = (id(10), id(20), id(30));
    let config = CallConfig {
        command_queue_capacity: 1,
        ..CallConfig::default()
    };
    let mut h = Harness::with_config(
        me,
        role,
        vec![(c1, ChannelKind::Voice, false, true)],
        config,
    );

    let call = h.join_with(c1, FakeIceFetcher::immediate()).unwrap();
    assert!(matches!(
        h.next_sent_command().await,
        Command::VoiceJoin { .. }
    ));
    h.deliver(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
        channel_id: c1.to_string(),
        peer_id: "p1".into(),
        video_codec: "vp8".into(),
        members: vec![VoiceState {
            user_id: me.to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        active_speakers: Vec::new(),
    })));
    // Fill the voice FIFO through the normal control path while the writer is paused.
    call.set_muted(false).unwrap();
    assert_eq!(call.set_muted(true), Err(CallError::Ended));
    assert_eq!(
        call.snapshot().phase,
        CallPhase::Ended(CallEndReason::CommandOverflow)
    );
    assert!(matches!(
        h.drain_sent_commands().as_slice(),
        [Command::VoiceLeave { .. }]
    ));

    // Reconnect never auto-rejoins.
    let gen2 = Generation::for_test(2);
    let res2 = voice_resources(gen2, me, role, vec![(c1, ChannelKind::Voice, false, true)]);
    h.calls.on_connection_status(
        gen2,
        ConnectionState::Online,
        &res2,
        Some(&tokio::runtime::Handle::current()),
    );
    assert!(h.calls.active_attempt_handle_parts().is_none());
}
