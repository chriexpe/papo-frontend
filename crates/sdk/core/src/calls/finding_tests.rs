//! Targeted tests for review findings 1–4: unsent-join rapid rejoin, concurrent control ordering,
//! monotonic local video-slot-plan revisions, and closed command-channel failure handling.

use super::{
    coordinator::SessionCalls,
    fake_media::{FakeMediaFactory, FakeVideoDestination},
    fifo::VoiceCommandReceiver,
    handle::CallHandle,
    lease::ClientCallLease,
    media_port::{
        IceServerConfig, InitialMediaSettings, MediaCommand, MediaEngineFactory, MediaLease,
        MediaLeaseError, MediaReport, MediaReportKind, MediaReportSink, MediaStartFuture,
        VideoDestinationId,
    },
    model::{CallAttemptId, CallConfig, CallEndReason, CallError, CallPhase, VideoKind},
    startup::{IceFetchFuture, IceFetcher},
};
use crate::{
    AccountId, ChannelKind, ChannelRecord, ConnectionState, Generation, Overwrite, OverwriteKey,
    ResourceSnapshot, ResourceTable, RoleFlags, RoleRecord, ServerAddress, ServerRecord,
    UserRecord, Uuid,
};
use papo_api::gateway::{
    Command, Event, KnownEvent, VoiceJoinedEvent, VoiceState, VoiceVideoRoute,
    VoiceVideoRoutesEvent,
};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn account(n: u128) -> AccountId {
    let server = ServerAddress::parse("https://voice.example").unwrap().id();
    AccountId::new(server, id(n))
}

struct ImmediateIce;
impl IceFetcher for ImmediateIce {
    fn fetch(&self, _generation: Generation) -> IceFetchFuture {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn voice_resources(
    generation: Generation,
    user: Uuid,
    role: Uuid,
    channel: Uuid,
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
            username: Some("caller".into()),
            nickname: None,
            banned: None,
            typing: None,
            created_at: None,
            roles: Some(vec![role]),
        },
    );
    snapshot.members.replace([user].into_iter());
    snapshot.channels = ResourceTable::from_rows([(
        channel,
        ChannelRecord {
            id: channel,
            name: Some("voice".into()),
            kind: ChannelKind::Voice,
            position: Some(1),
            topic: None,
            parent_id: None,
            permissions: vec![Overwrite {
                key: OverwriteKey::Canonical(role),
                read_channel: false,
                send_messages: Some(false),
                delete_messages: Some(false),
                connect_voice: Some(true),
            }],
            created_at: None,
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            notification_settings: None,
        },
    )]);
    Arc::new(snapshot)
}

async fn next_sent(
    calls: &SessionCalls,
    gen_id: Generation,
    rx: &mut VoiceCommandReceiver,
) -> Command {
    loop {
        let item = rx.recv().await.expect("expected voice command");
        if calls.should_send(&item, gen_id) {
            return item.command;
        }
    }
}

fn deliver_joined(
    calls: &Arc<SessionCalls>,
    gen_id: Generation,
    channel: Uuid,
    peer_id: &str,
    members: Vec<VoiceState>,
    rt: &tokio::runtime::Handle,
) {
    calls.on_gateway_event(
        gen_id,
        &Event::Known(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
            channel_id: channel.to_string(),
            peer_id: peer_id.into(),
            video_codec: "h264".into(),
            members,
            active_speakers: Vec::new(),
        }))),
        rt,
    );
}

#[tokio::test(start_paused = true)]
async fn replacing_call_before_socket_drains_voice_join_does_not_swallow_replacement_ack() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let ch = id(100);
    let calls = SessionCalls::new(
        acct,
        gen1,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &voice_resources(gen1, acct.user(), id(20), ch),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    // Call A reaches Joining (VoiceJoin(A) is queued in FIFO, NOT yet polled by AccountSocket).
    let (a1, c1) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            Arc::new(ImmediateIce),
            &rt,
            &root,
        )
        .unwrap();
    let call_a = CallHandle::from_parts(acct, a1, Arc::clone(&calls), c1, rt.clone());
    while call_a.snapshot().phase != CallPhase::Joining {
        tokio::task::yield_now().await;
    }

    // Immediately replace A with B on the SAME channel before AccountSocket drains A's VoiceJoin.
    let (a2, c2) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            Arc::new(ImmediateIce),
            &rt,
            &root,
        )
        .unwrap();
    let call_b = CallHandle::from_parts(acct, a2, Arc::clone(&calls), c2, rt.clone());
    assert_eq!(
        call_a.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );

    // Only B's VoiceJoin is transmitted (A's unsent VoiceJoin is dropped and NO VoiceLeave is queued).
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    assert!(rx.try_recv().is_none());

    // B's voice_joined is NOT swallowed by an abandoned-join counter.
    deliver_joined(
        &calls,
        gen1,
        ch,
        "peer-b",
        vec![VoiceState {
            user_id: acct.user().to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        &rt,
    );
    assert_eq!(call_b.snapshot().phase, CallPhase::Joined);
    assert_eq!(call_b.snapshot().peer_id.as_deref(), Some("peer-b"));
}

/// Custom lease that pauses the first `SetMuted` call via a barrier to force concurrent dispatch overlap.
struct InterleavingLease {
    attempt: CallAttemptId,
    reports: MediaReportSink,
    stopped: AtomicBool,
    commands: Arc<Mutex<Vec<MediaCommand>>>,
    pause_first_mute: Arc<AtomicBool>,
    first_mute_entered: Arc<Barrier>,
    release_first_mute: Arc<Barrier>,
}

impl MediaLease for InterleavingLease {
    fn send(&self, command: MediaCommand) -> Result<(), MediaLeaseError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(MediaLeaseError::Closed);
        }
        let should_pause = matches!(command, MediaCommand::SetMuted { .. })
            && self.pause_first_mute.swap(false, Ordering::AcqRel);
        if should_pause {
            self.first_mute_entered.wait();
            self.release_first_mute.wait();
        }
        self.commands.lock().unwrap().push(command);
        Ok(())
    }

    fn stop(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            self.reports
                .try_send(MediaReport {
                    attempt: self.attempt,
                    kind: MediaReportKind::Drained,
                })
                .unwrap();
        }
    }
}

struct InterleavingFactory {
    commands: Arc<Mutex<Vec<MediaCommand>>>,
    pause_first_mute: Arc<AtomicBool>,
    first_mute_entered: Arc<Barrier>,
    release_first_mute: Arc<Barrier>,
}

impl MediaEngineFactory for InterleavingFactory {
    fn start(
        &self,
        attempt: CallAttemptId,
        _ice_servers: Vec<IceServerConfig>,
        _settings: InitialMediaSettings,
        reports: MediaReportSink,
    ) -> MediaStartFuture {
        let lease = InterleavingLease {
            attempt,
            reports,
            stopped: AtomicBool::new(false),
            commands: Arc::clone(&self.commands),
            pause_first_mute: Arc::clone(&self.pause_first_mute),
            first_mute_entered: Arc::clone(&self.first_mute_entered),
            release_first_mute: Arc::clone(&self.release_first_mute),
        };
        Box::pin(async move { Ok(Box::new(lease) as Box<dyn MediaLease>) })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_controls_execute_wire_and_media_commands_in_exact_state_order() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let ch = id(100);
    let calls = SessionCalls::new(
        acct,
        gen1,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &voice_resources(gen1, acct.user(), id(20), ch),
        Some(&rt),
    );
    let commands = Arc::new(Mutex::new(Vec::new()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let factory: Arc<dyn MediaEngineFactory> = Arc::new(InterleavingFactory {
        commands: Arc::clone(&commands),
        pause_first_mute: Arc::new(AtomicBool::new(true)),
        first_mute_entered: Arc::clone(&entered),
        release_first_mute: Arc::clone(&release),
    });
    let root = CancellationToken::new();
    let (attempt, cell) = calls
        .join(ch, Some(factory), Arc::new(ImmediateIce), &rt, &root)
        .unwrap();
    let handle = CallHandle::from_parts(acct, attempt, Arc::clone(&calls), cell, rt.clone());
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(
        &calls,
        gen1,
        ch,
        "peer-1",
        vec![VoiceState {
            user_id: acct.user().to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        &rt,
    );

    // Thread 1 unmutes and pauses inside MediaLease::send(SetMuted(false)).
    let h1 = handle.clone();
    let t1 = std::thread::spawn(move || h1.set_muted(false).unwrap());
    entered.wait();

    // Thread 2 loses audio focus while Thread 1 is mid-dispatch; it must NOT overtake Thread 1.
    let h2 = handle.clone();
    let t2 = std::thread::spawn(move || h2.on_audio_focus_lost().unwrap());
    // Give Thread 2 time to mutate state (muted = true) and wait on media_dispatch.
    while !handle.snapshot().settings.muted {
        std::thread::yield_now();
    }
    release.wait();
    t1.join().unwrap();
    t2.join().unwrap();

    // Snapshot, wire FIFO, and MediaLease must ALL end in muted=true with SetMuted(false) first.
    assert!(handle.snapshot().settings.muted);
    let mutes: Vec<bool> = commands
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            MediaCommand::SetMuted { muted } => Some(*muted),
            _ => None,
        })
        .collect();
    assert_eq!(mutes, vec![false, true]);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceMute { muted: false, .. }
    ));
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceMute { muted: true, .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn local_destination_bind_and_unbind_increment_slot_plan_revision_monotonically() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let alice = id(11);
    let ch = id(100);
    let calls = SessionCalls::new(
        acct,
        gen1,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &voice_resources(gen1, acct.user(), id(20), ch),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (attempt, cell) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            Arc::new(ImmediateIce),
            &rt,
            &root,
        )
        .unwrap();
    let handle = CallHandle::from_parts(acct, attempt, Arc::clone(&calls), cell, rt.clone());
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(
        &calls,
        gen1,
        ch,
        "p1",
        vec![
            VoiceState {
                user_id: acct.user().to_string(),
                muted: true,
                camera_on: false,
                screen_sharing: false,
            },
            VoiceState {
                user_id: alice.to_string(),
                muted: false,
                camera_on: true,
                screen_sharing: false,
            },
        ],
        &rt,
    );

    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceVideoRoutes(Box::new(
            VoiceVideoRoutesEvent {
                channel_id: ch.to_string(),
                peer_id: "p1".into(),
                revision: 1,
                routes: vec![VoiceVideoRoute {
                    track_id: "papo-video-0".into(),
                    user_id: alice.to_string(),
                    kind: "video".into(),
                    first_seq: None,
                    rtp_route_id: Some(1),
                    rtp_route_ext_id: Some(14),
                }],
            },
        ))),
        &rt,
    );
    let dest1 = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(1)));
    let dest2 = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(1)));
    handle
        .bind_video(alice, VideoKind::Camera, dest1.clone())
        .unwrap();
    handle
        .bind_video(alice, VideoKind::Camera, dest2.clone())
        .unwrap();
    assert!(dest1.is_revoked());
    assert!(!dest2.is_revoked());
    handle.unbind_video(alice, VideoKind::Camera).unwrap();
    assert!(dest2.is_revoked());

    let lease = media.latest_lease().unwrap();
    let revisions: Vec<u64> = lease
        .commands()
        .iter()
        .filter_map(|cmd| match cmd {
            MediaCommand::SetVideoSlotPlan { revision, .. } => Some(*revision),
            _ => None,
        })
        .collect();
    assert_eq!(revisions, vec![1, 2, 3, 4]);
}

#[tokio::test(start_paused = true)]
async fn closed_command_channel_fails_active_attempt_instead_of_silently_dropping() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let ch = id(100);
    let calls = SessionCalls::new(
        acct,
        gen1,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &voice_resources(gen1, acct.user(), id(20), ch),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (attempt, cell) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            Arc::new(ImmediateIce),
            &rt,
            &root,
        )
        .unwrap();
    let handle = CallHandle::from_parts(acct, attempt, Arc::clone(&calls), cell, rt.clone());
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(
        &calls,
        gen1,
        ch,
        "p1",
        vec![VoiceState {
            user_id: acct.user().to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        &rt,
    );

    // Drop the FIFO receiver to simulate a closed command channel before connection-loss arrives.
    drop(rx);
    assert_eq!(handle.set_muted(false), Err(CallError::Ended));
    assert_eq!(
        handle.snapshot().phase,
        CallPhase::Ended(CallEndReason::ConnectionLost)
    );
}
