//! Deterministic regression tests for edge cases found in the Session 14a review pass.

use super::{
    coordinator::SessionCalls,
    fake_media::{FakeMediaFactory, FakeStartBehavior, FakeVideoDestination},
    fifo::VoiceCommandReceiver,
    handle::CallHandle,
    lease::ClientCallLease,
    media_port::{
        IceServerConfig, MediaCommand, MediaEngineFactory, MediaReportKind, VideoDestinationId,
    },
    model::{
        CallConfig, CallEndReason, CallPhase, CallWarning, CameraFacing, PlaybackSettings,
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
    Command, ErrorEvent, Event, KnownEvent, VoiceAnswerEvent, VoiceJoinedEvent, VoiceLeaveEvent,
    VoiceState, VoiceVideoRoute, VoiceVideoRoutesEvent,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
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

enum IceStep {
    Ok,
    Gated(oneshot::Receiver<Result<Vec<IceServerConfig>, ClientError>>),
}

struct ScriptedIce(Mutex<VecDeque<IceStep>>);

impl ScriptedIce {
    fn immediate() -> Arc<Self> {
        Arc::new(Self(Mutex::new(VecDeque::new())))
    }

    fn gated() -> (
        Arc<Self>,
        oneshot::Sender<Result<Vec<IceServerConfig>, ClientError>>,
    ) {
        let (tx, rx) = oneshot::channel();
        (
            Arc::new(Self(Mutex::new(VecDeque::from([IceStep::Gated(rx)])))),
            tx,
        )
    }
}

impl IceFetcher for ScriptedIce {
    fn fetch(&self, _generation: Generation) -> IceFetchFuture {
        let step = self.0.lock().unwrap().pop_front().unwrap_or(IceStep::Ok);
        Box::pin(async move {
            match step {
                IceStep::Ok => Ok(Vec::new()),
                IceStep::Gated(rx) => rx.await.unwrap_or(Err(ClientError::Closed)),
            }
        })
    }
}

fn resources(
    generation: Generation,
    user: Uuid,
    role: Uuid,
    channels: &[Uuid],
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
    snapshot.channels = ResourceTable::from_rows(channels.iter().enumerate().map(|(i, &ch)| {
        (
            ch,
            ChannelRecord {
                id: ch,
                name: Some(format!("voice-{i}")),
                kind: ChannelKind::Voice,
                position: Some(i64::try_from(i).unwrap_or(0)),
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
        )
    }));
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

#[tokio::test(start_paused = true)]
async fn rejoin_during_preparing_before_media_start_never_wedges_lease() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let (ch_a, ch_b) = (id(100), id(101));
    let lease = Arc::new(ClientCallLease::default());
    let calls = SessionCalls::new(acct, gen1, CallConfig::default(), Arc::clone(&lease));
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &resources(gen1, acct.user(), id(20), &[ch_a, ch_b]),
        Some(&rt),
    );

    let (ice_gated, _ice_tx) = ScriptedIce::gated();
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    // Attempt 1 pauses in Preparing (ICE fetch in flight, media_factory.start never called).
    let (a1, c1) = calls
        .join(
            ch_a,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ice_gated,
            &rt,
            &root,
        )
        .unwrap();
    let h1 = CallHandle::from_parts(acct, a1, Arc::clone(&calls), c1, rt.clone());
    assert_eq!(h1.snapshot().phase, CallPhase::Preparing);

    // Same-account rejoin to ch_b must NOT wait for attempt 1 drain or wedge ClientCallLease.
    let (a2, c2) = calls
        .join(
            ch_b,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ScriptedIce::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let h2 = CallHandle::from_parts(acct, a2, Arc::clone(&calls), c2, rt.clone());
    assert_eq!(
        h1.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    assert!(matches!(
        tokio::time::timeout(CallConfig::default().drain_timeout, next_sent(&calls, gen1, &mut rx)).await.unwrap(),
        Command::VoiceJoin { ref channel_id } if channel_id == &ch_b.to_string()
    ));
    assert_eq!(h2.snapshot().phase, CallPhase::Joining);
    assert!(!lease.is_release_pending());
}

#[tokio::test(start_paused = true)]
async fn leave_before_socket_drains_voice_join_skips_wire_leave_and_preserves_next_join() {
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
        &resources(gen1, acct.user(), id(20), &[ch]),
        Some(&rt),
    );

    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (a1, c1) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ScriptedIce::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let h1 = CallHandle::from_parts(acct, a1, Arc::clone(&calls), c1, rt.clone());
    // Let startup finish queueing VoiceJoin, but do NOT pop it from rx yet!
    while h1.snapshot().phase != CallPhase::Joining {
        tokio::task::yield_now().await;
    }

    // Leave before AccountSocket drains VoiceJoin -> both VoiceJoin and paired VoiceLeave are skipped.
    h1.leave();
    while let Some(item) = rx.try_recv() {
        assert!(!calls.should_send(&item, gen1));
    }

    // Rejoining the same channel must NOT have its voice_joined swallowed by a phantom abandoned join.
    let (a2, c2) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ScriptedIce::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let h2 = CallHandle::from_parts(acct, a2, Arc::clone(&calls), c2, rt.clone());
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
            channel_id: ch.to_string(),
            peer_id: "peer-live".into(),
            video_codec: "h264".into(),
            members: vec![VoiceState {
                user_id: acct.user().to_string(),
                muted: true,
                camera_on: false,
                screen_sharing: false,
            }],
            active_speakers: Vec::new(),
        }))),
        &rt,
    );
    assert_eq!(h2.snapshot().phase, CallPhase::Joined);
    assert_eq!(h2.snapshot().peer_id.as_deref(), Some("peer-live"));
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

fn deliver_error(
    calls: &Arc<SessionCalls>,
    generation: Generation,
    code: &str,
    channel: Option<Uuid>,
    rt: &tokio::runtime::Handle,
) {
    calls.on_gateway_event(
        generation,
        &Event::Known(KnownEvent::Error(Box::new(ErrorEvent {
            message: code.into(),
            code: Some(code.into()),
            channel_id: channel.map(|id| id.to_string()),
            peer_id: None,
        }))),
        rt,
    );
}

fn deliver_self_leave(
    calls: &Arc<SessionCalls>,
    generation: Generation,
    channel: Uuid,
    peer_id: &str,
    rt: &tokio::runtime::Handle,
) {
    calls.on_gateway_event(
        generation,
        &Event::Known(KnownEvent::VoiceLeave(Box::new(VoiceLeaveEvent {
            channel_id: channel.to_string(),
            user_id: calls.account.user().to_string(),
            peer_id: peer_id.into(),
        }))),
        rt,
    );
}

fn join_now(
    calls: &Arc<SessionCalls>,
    media: &Arc<FakeMediaFactory>,
    channel: Uuid,
    root: &CancellationToken,
    rt: &tokio::runtime::Handle,
) -> CallHandle {
    let (attempt, cell) = calls
        .join(
            channel,
            Some(Arc::clone(media) as Arc<dyn MediaEngineFactory>),
            ScriptedIce::immediate(),
            rt,
            root,
        )
        .unwrap();
    CallHandle::from_parts(calls.account, attempt, Arc::clone(calls), cell, rt.clone())
}

#[tokio::test(start_paused = true)]
async fn rejoin_after_join_timeout_and_room_closed_not_swallowed_by_abandoned_join() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let (ch_a, ch_b) = (id(100), id(101));
    let config = CallConfig {
        join_timeout: Duration::from_millis(200),
        ..CallConfig::default()
    };
    let calls = SessionCalls::new(acct, gen1, config, Arc::new(ClientCallLease::default()));
    let mut rx = calls.attach_socket(gen1);
    calls.on_connection_status(
        gen1,
        ConnectionState::Online,
        &resources(gen1, acct.user(), id(20), &[ch_a, ch_b]),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    // 1. Join ch_a and time out (voice_joined never arrived).
    let h1 = join_now(&calls, &media, ch_a, &root, &rt);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    tokio::time::advance(Duration::from_millis(250)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        h1.snapshot().phase,
        CallPhase::Ended(CallEndReason::JoinTimeout)
    );
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    // If the first join never acquired a peer, the ordered leave returns not-found.
    deliver_error(&calls, gen1, "voice-not-found", None, &rt);
    assert!(calls.state.lock().unwrap().abandoned_joins.is_empty());

    // 2. Rejoin ch_a -> leave while Joining (creates abandoned join on ch_a), then join ch_b.
    let h2 = join_now(&calls, &media, ch_a, &root, &rt);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let h3 = join_now(&calls, &media, ch_b, &root, &rt);
    assert_eq!(
        h2.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(
        &calls,
        gen1,
        ch_b,
        "peer-b",
        vec![VoiceState {
            user_id: acct.user().to_string(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        }],
        &rt,
    );
    assert_eq!(h3.snapshot().phase, CallPhase::Joined);

    // 3. A room closure ends the current joined call.
    deliver_error(&calls, gen1, "voice-room-closed", Some(ch_b), &rt);
    assert_eq!(
        h3.snapshot().phase,
        CallPhase::Ended(CallEndReason::RoomClosed)
    );

    // The same channel-scoped closure also applies to a join awaiting its acknowledgement.
    let _h4 = join_now(&calls, &media, ch_a, &root, &rt);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let h5 = join_now(&calls, &media, ch_a, &root, &rt);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_error(&calls, gen1, "voice-room-closed", Some(ch_a), &rt);
    assert_eq!(
        h5.snapshot().phase,
        CallPhase::Ended(CallEndReason::RoomClosed)
    );
}

#[tokio::test(start_paused = true)]
async fn late_join_reply_after_timeout_cannot_admit_replacement() {
    let rt = tokio::runtime::Handle::current();
    let generation = Generation::for_test(1);
    let acct = account(10);
    let channel = id(100);
    let config = CallConfig {
        join_timeout: Duration::from_millis(200),
        ..CallConfig::default()
    };
    let calls = SessionCalls::new(
        acct,
        generation,
        config,
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(generation);
    calls.on_connection_status(
        generation,
        ConnectionState::Online,
        &resources(generation, acct.user(), id(20), &[channel]),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    let old = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    tokio::time::advance(config.join_timeout + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        old.snapshot().phase,
        CallPhase::Ended(CallEndReason::JoinTimeout)
    );

    let new = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    media.wait_lease(2).await;
    assert_eq!(new.snapshot().phase, CallPhase::Joining);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(&calls, generation, channel, "peer-old", Vec::new(), &rt);
    assert_eq!(new.snapshot().peer_id, None);
    deliver_joined(&calls, generation, channel, "peer-new", Vec::new(), &rt);
    assert_eq!(new.snapshot().peer_id.as_deref(), Some("peer-new"));
}

#[tokio::test(start_paused = true)]
async fn stale_negotiation_error_does_not_consume_abandoned_join_or_end_replacement() {
    let rt = tokio::runtime::Handle::current();
    let generation = Generation::for_test(1);
    let acct = account(10);
    let channel = id(100);
    let calls = SessionCalls::new(
        acct,
        generation,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(generation);
    calls.on_connection_status(
        generation,
        ConnectionState::Online,
        &resources(generation, acct.user(), id(20), &[channel]),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    let old = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let new = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    assert_eq!(
        old.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );

    for peer in ["peer-old", "peer-new"] {
        deliver_error(&calls, generation, "voice-invalid-sdp", None, &rt);
        assert_eq!(new.snapshot().phase, CallPhase::Joining);
        deliver_joined(&calls, generation, channel, peer, Vec::new(), &rt);
        if peer == "peer-old" {
            assert_eq!(new.snapshot().phase, CallPhase::Joining);
        }
    }
    assert_eq!(new.snapshot().peer_id.as_deref(), Some("peer-new"));
}

#[tokio::test(start_paused = true)]
async fn rejected_join_and_its_leave_do_not_consume_the_next_abandoned_join() {
    let rt = tokio::runtime::Handle::current();
    let generation = Generation::for_test(1);
    let acct = account(10);
    let (first_channel, next_channel) = (id(100), id(101));
    let calls = SessionCalls::new(
        acct,
        generation,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(generation);
    calls.on_connection_status(
        generation,
        ConnectionState::Online,
        &resources(
            generation,
            acct.user(),
            id(20),
            &[first_channel, next_channel],
        ),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    let _known = join_now(&calls, &media, first_channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(
        &calls,
        generation,
        first_channel,
        "peer-known",
        Vec::new(),
        &rt,
    );
    let _first = join_now(&calls, &media, first_channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let _second = join_now(&calls, &media, next_channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let current = join_now(&calls, &media, next_channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));

    deliver_error(&calls, generation, "voice-forbidden", None, &rt);
    assert_eq!(current.snapshot().phase, CallPhase::Joining);
    deliver_self_leave(&calls, generation, first_channel, "peer-known", &rt);
    deliver_error(&calls, generation, "voice-not-found", None, &rt);
    deliver_joined(
        &calls,
        generation,
        next_channel,
        "peer-second",
        Vec::new(),
        &rt,
    );
    assert_eq!(current.snapshot().phase, CallPhase::Joining);
    deliver_joined(
        &calls,
        generation,
        next_channel,
        "peer-current",
        Vec::new(),
        &rt,
    );
    assert_eq!(current.snapshot().peer_id.as_deref(), Some("peer-current"));
}

#[tokio::test(start_paused = true)]
async fn delayed_leave_for_known_peer_preserves_newer_abandoned_join() {
    let rt = tokio::runtime::Handle::current();
    let generation = Generation::for_test(1);
    let acct = account(10);
    let channel = id(100);
    let calls = SessionCalls::new(
        acct,
        generation,
        CallConfig::default(),
        Arc::new(ClientCallLease::default()),
    );
    let mut rx = calls.attach_socket(generation);
    calls.on_connection_status(
        generation,
        ConnectionState::Online,
        &resources(generation, acct.user(), id(20), &[channel]),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();

    let _first = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(&calls, generation, channel, "peer-first", Vec::new(), &rt);
    let _second = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    let current = join_now(&calls, &media, channel, &root, &rt);
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceLeave { .. }
    ));
    assert!(matches!(
        next_sent(&calls, generation, &mut rx).await,
        Command::VoiceJoin { .. }
    ));

    deliver_self_leave(&calls, generation, channel, "peer-first", &rt);
    deliver_joined(&calls, generation, channel, "peer-second", Vec::new(), &rt);
    assert_eq!(current.snapshot().phase, CallPhase::Joining);
    deliver_joined(&calls, generation, channel, "peer-current", Vec::new(), &rt);
    assert_eq!(current.snapshot().peer_id.as_deref(), Some("peer-current"));
    deliver_error(&calls, generation, "voice-rate-limited", None, &rt);
    assert!(
        current
            .snapshot()
            .warnings
            .contains(&CallWarning::RateLimited)
    );
}

const VIDEO_TEST_GENERATION: Generation = Generation::for_test(1);
const VIDEO_TEST_PARTICIPANT: Uuid = Uuid::from_u128(11);
const VIDEO_TEST_CHANNEL: Uuid = Uuid::from_u128(100);

struct VideoTestCall {
    calls: Arc<SessionCalls>,
    handle: CallHandle,
    media: Arc<FakeMediaFactory>,
    rx: VoiceCommandReceiver,
}

async fn joined_video_receiver() -> VideoTestCall {
    let rt = tokio::runtime::Handle::current();
    let gen1 = VIDEO_TEST_GENERATION;
    let acct = account(10);
    let alice = VIDEO_TEST_PARTICIPANT;
    let ch = VIDEO_TEST_CHANNEL;
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
        &resources(gen1, acct.user(), id(20), &[ch]),
        Some(&rt),
    );
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let handle = join_now(&calls, &media, ch, &root, &rt);
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
            user_id: alice.to_string(),
            muted: false,
            camera_on: true,
            screen_sharing: false,
        }],
        &rt,
    );
    media
        .latest_lease()
        .unwrap()
        .emit(MediaReportKind::LocalOffer {
            sdp: "v=0\r\n".into(),
        });
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceOffer { .. }
    ));
    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceAnswer(Box::new(VoiceAnswerEvent {
            channel_id: ch.to_string(),
            peer_id: "p1".into(),
            sdp: "v=0\r\n".into(),
        }))),
        &rt,
    );

    VideoTestCall {
        calls,
        handle,
        media,
        rx,
    }
}

#[tokio::test(start_paused = true)]
async fn unsubscribe_race_with_stale_video_routes_never_sticks_target_requested() {
    let VideoTestCall {
        calls,
        handle,
        media,
        mut rx,
    } = joined_video_receiver().await;
    let lease = media.latest_lease().unwrap();
    let rt = tokio::runtime::Handle::current();
    let gen1 = VIDEO_TEST_GENERATION;
    let ch = VIDEO_TEST_CHANNEL;
    let alice = VIDEO_TEST_PARTICIPANT;
    let target = VideoTarget {
        participant: alice,
        kind: VideoKind::Camera,
    };
    handle.set_video_interest(vec![target]).unwrap();
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::TrackSubscribe { .. }
    ));
    handle
        .bind_video(
            alice,
            VideoKind::Camera,
            Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(1))),
        )
        .unwrap();
    handle.set_playback(alice, 140, false).unwrap();
    let route = VoiceVideoRoute {
        track_id: "papo-video-0".into(),
        user_id: alice.to_string(),
        kind: "video".into(),
        first_seq: None,
        rtp_route_id: Some(1),
        rtp_route_ext_id: Some(14),
    };
    deliver_video_routes(&calls, gen1, ch, "p1", 1, vec![route.clone()], &rt);
    let plan = |commands: Vec<MediaCommand>| {
        commands
            .into_iter()
            .rev()
            .find_map(|command| match command {
                MediaCommand::SetVideoSlotPlan { revision, slots } => Some((revision, slots)),
                _ => None,
            })
            .unwrap()
    };
    let (visible_revision, visible_slots) = plan(lease.commands());
    assert!(visible_slots[0].destination.is_some());
    let count = lease.commands().len();
    handle.set_video_interest(vec![target, target]).unwrap();
    assert_eq!(lease.commands().len(), count);
    handle.set_video_interest(Vec::new()).unwrap();
    let (hidden_revision, hidden_slots) = plan(lease.commands());
    assert!(hidden_revision > visible_revision);
    assert!(hidden_slots[0].destination.is_none());
    assert!(handle.snapshot().video_bindings[0].destination_id.is_none());
    assert_eq!(
        handle.snapshot().playback.get(&alice),
        Some(&PlaybackSettings::new(140, false))
    );
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::TrackUnsubscribe { .. }
    ));
    let mut late_route = route.clone();
    late_route.rtp_route_id = Some(2);
    deliver_video_routes(&calls, gen1, ch, "p1", 2, vec![late_route], &rt);
    let (late_revision, late_slots) = plan(lease.commands());
    assert!(late_revision > hidden_revision);
    assert!(late_slots[0].destination.is_none());

    deliver_video_routes(&calls, gen1, ch, "p1", 3, Vec::new(), &rt);

    // Re-requesting Alice after rate-limit interval MUST emit TrackSubscribe again.
    tokio::time::advance(Duration::from_millis(250)).await;
    handle.set_video_interest(vec![target]).unwrap();
    let command = tokio::time::timeout(Duration::from_secs(1), next_sent(&calls, gen1, &mut rx));
    assert!(matches!(
        command.await.unwrap(),
        Command::TrackSubscribe { .. }
    ));
    deliver_video_routes(&calls, gen1, ch, "p1", 4, vec![route], &rt);
    let (shown_revision, shown_slots) = plan(lease.commands());
    assert!(shown_revision > late_revision);
    assert!(shown_slots[0].destination.is_some());
    assert_eq!(
        handle.snapshot().playback.get(&alice),
        Some(&PlaybackSettings::new(140, false))
    );
}

fn deliver_video_routes(
    calls: &Arc<SessionCalls>,
    gen_id: Generation,
    channel: Uuid,
    peer_id: &str,
    revision: u64,
    routes: Vec<VoiceVideoRoute>,
    rt: &tokio::runtime::Handle,
) {
    calls.on_gateway_event(
        gen_id,
        &Event::Known(KnownEvent::VoiceVideoRoutes(Box::new(
            VoiceVideoRoutesEvent {
                channel_id: channel.to_string(),
                peer_id: peer_id.into(),
                revision,
                routes,
            },
        ))),
        rt,
    );
}

#[tokio::test(start_paused = true)]
async fn controls_during_in_flight_media_start_sync_to_lease_on_install() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let remote = id(11);
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
        &resources(gen1, acct.user(), id(20), &[ch]),
        Some(&rt),
    );

    let media = Arc::new(FakeMediaFactory::new());
    let (gate_tx, gate_rx) = oneshot::channel();
    media.push_start(FakeStartBehavior::Gated(gate_rx));
    let root = CancellationToken::new();

    let (attempt, cell) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ScriptedIce::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let handle = CallHandle::from_parts(acct, attempt, Arc::clone(&calls), cell, rt.clone());
    media.wait_starts(1).await;

    // Issue controls while media_factory.start() is still in flight.
    handle.set_muted(false).unwrap();
    handle.set_camera(true).unwrap();
    handle.set_camera_facing(CameraFacing::Back).unwrap();
    handle.set_playback(remote, 140, true).unwrap();

    // Release media startup -> lease receives synced controls immediately upon installation.
    let _ = gate_tx.send(Ok(()));
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    deliver_joined(&calls, gen1, ch, "peer-live", Vec::new(), &rt);
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceMute { muted: false, .. }
    ));
    assert!(matches!(
        next_sent(&calls, gen1, &mut rx).await,
        Command::VoiceCamera { on: true, .. }
    ));
    let lease = media.latest_lease().unwrap();
    let cmds = lease.commands();
    assert!(matches!(cmds.as_slice(), [
        MediaCommand::SetMuted { muted: false },
        MediaCommand::SetCameraFacing {
            facing: CameraFacing::Back
        },
        MediaCommand::SetCamera { on: true },
        MediaCommand::SetPlayback {
            user,
            playback: PlaybackSettings {
                gain_percent: 140,
                muted: true
            }
        },
        MediaCommand::StartNegotiation { .. },
    ] if *user == remote));
}
