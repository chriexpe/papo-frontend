//! Additional deterministic call lifecycle tests: pre-join leave vs post-join leave,
//! `ReleasePending` drain timeout recovery, and post-join subscribe retry backoff.

use super::{
    coordinator::SessionCalls,
    fake_media::FakeMediaFactory,
    fifo::VoiceCommandReceiver,
    handle::CallHandle,
    lease::ClientCallLease,
    media_port::{IceServerConfig, MediaEngineFactory, MediaReportKind},
    model::{CallConfig, CallEndReason, CallError, CallPhase, VideoKind, VideoTarget},
    startup::{IceFetchFuture, IceFetcher},
};
use crate::{
    AccountId, ChannelKind, ChannelRecord, ClientError, ConnectionState, Generation, Overwrite,
    OverwriteKey, ResourceSnapshot, ResourceTable, RoleFlags, RoleRecord, ServerAddress,
    ServerRecord, UserRecord, Uuid,
};
use papo_api::gateway::{
    Command, ErrorEvent, Event, KnownEvent, VoiceAnswerEvent, VoiceJoinedEvent, VoiceState,
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
            status_message: None,
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

async fn drain_one(
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
async fn leave_before_voice_join_sends_no_leave_and_leave_after_voice_join_ignores_late_joined() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let ch = id(100);
    let lease = Arc::new(ClientCallLease::default());
    let calls = SessionCalls::new(acct, gen1, CallConfig::default(), lease);
    let mut rx = calls.attach_socket(gen1);
    let resources = voice_resources(gen1, acct.user(), id(20), ch);
    calls.on_connection_status(gen1, ConnectionState::Online, &resources, Some(&rt));

    // Case 1: leave while still in Preparing (before voice_join is sent) -> no voice_leave emitted.
    let (ice_gated, ice_tx) = FakeIceFetcher::with_gate();
    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (attempt1, cell1) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            ice_gated,
            &rt,
            &root,
        )
        .unwrap();
    let handle1 = CallHandle::from_parts(acct, attempt1, Arc::clone(&calls), cell1, rt.clone());
    assert_eq!(handle1.snapshot().phase, CallPhase::Preparing);
    handle1.leave();
    assert_eq!(
        handle1.snapshot().phase,
        CallPhase::Ended(CallEndReason::LocalLeave)
    );
    let _ = ice_tx.send(Ok(Vec::new()));
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_none());

    // Case 2: leave AFTER voice_join is sent while still in Joining -> voice_leave IS emitted,
    // and a late voice_joined for that abandoned join is silently ignored.
    let (attempt2, cell2) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            FakeIceFetcher::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let handle2 = CallHandle::from_parts(acct, attempt2, Arc::clone(&calls), cell2, rt.clone());
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));
    assert_eq!(handle2.snapshot().phase, CallPhase::Joining);

    handle2.leave();
    assert_eq!(
        handle2.snapshot().phase,
        CallPhase::Ended(CallEndReason::LocalLeave)
    );
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceLeave { .. }
    ));

    // Late voice_joined for the abandoned join must NOT revive the call or start a new active call.
    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
            channel_id: ch.to_string(),
            peer_id: "peer-abandoned".into(),
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
    assert!(calls.active_attempt_handle_parts().is_none());
}

#[tokio::test(start_paused = true)]
async fn release_pending_blocks_new_join_until_late_drain_completes() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let ch = id(100);
    let lease = Arc::new(ClientCallLease::default());
    let config = CallConfig {
        drain_timeout: Duration::from_millis(200),
        ..CallConfig::default()
    };
    let calls = SessionCalls::new(acct, gen1, config, Arc::clone(&lease));
    let mut rx = calls.attach_socket(gen1);
    let resources = voice_resources(gen1, acct.user(), id(20), ch);
    calls.on_connection_status(gen1, ConnectionState::Online, &resources, Some(&rt));

    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (attempt1, cell1) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            FakeIceFetcher::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let handle1 = CallHandle::from_parts(acct, attempt1, Arc::clone(&calls), cell1, rt.clone());
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));

    // Hold drain past drain_timeout to enter ReleasePending.
    let lease1 = media.latest_lease().unwrap();
    lease1.set_auto_drain(false);
    handle1.leave();
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceLeave { .. }
    ));

    tokio::time::advance(Duration::from_millis(250)).await;
    tokio::task::yield_now().await;

    assert!(matches!(
        calls.join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            FakeIceFetcher::immediate(),
            &rt,
            &root,
        ),
        Err(CallError::ReleasePending)
    ));

    // Late Drained report clears ReleasePending so a fresh join succeeds.
    lease1.emit(MediaReportKind::Drained);
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let (attempt2, _) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            FakeIceFetcher::immediate(),
            &rt,
            &root,
        )
        .expect("join should succeed after late drain completes");
    assert_ne!(attempt1, attempt2);
}

#[tokio::test(start_paused = true)]
async fn post_join_subscribe_rejection_retries_with_backoff_on_fake_clock() {
    let rt = tokio::runtime::Handle::current();
    let gen1 = Generation::for_test(1);
    let acct = account(10);
    let remote = id(11);
    let ch = id(100);
    let lease = Arc::new(ClientCallLease::default());
    let calls = SessionCalls::new(acct, gen1, CallConfig::default(), lease);
    let mut rx = calls.attach_socket(gen1);
    let resources = voice_resources(gen1, acct.user(), id(20), ch);
    calls.on_connection_status(gen1, ConnectionState::Online, &resources, Some(&rt));

    let media = Arc::new(FakeMediaFactory::new());
    let root = CancellationToken::new();
    let (attempt, cell) = calls
        .join(
            ch,
            Some(Arc::clone(&media) as Arc<dyn MediaEngineFactory>),
            FakeIceFetcher::immediate(),
            &rt,
            &root,
        )
        .unwrap();
    let handle = CallHandle::from_parts(acct, attempt, Arc::clone(&calls), cell, rt.clone());
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceJoin { .. }
    ));

    // Admit join with remote publishing camera, complete initial answer.
    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceJoined(Box::new(VoiceJoinedEvent {
            channel_id: ch.to_string(),
            peer_id: "peer-sub".into(),
            video_codec: "h264".into(),
            members: vec![
                VoiceState {
                    user_id: acct.user().to_string(),
                    muted: true,
                    camera_on: false,
                    screen_sharing: false,
                },
                VoiceState {
                    user_id: remote.to_string(),
                    muted: false,
                    camera_on: true,
                    screen_sharing: false,
                },
            ],
            active_speakers: Vec::new(),
        }))),
        &rt,
    );
    let lease_handle = media.latest_lease().unwrap();
    lease_handle.emit(MediaReportKind::LocalOffer {
        sdp: "v=0\r\n".into(),
    });
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::VoiceOffer { .. }
    ));
    calls.on_gateway_event(
        gen1,
        &Event::Known(KnownEvent::VoiceAnswer(Box::new(VoiceAnswerEvent {
            channel_id: ch.to_string(),
            peer_id: "peer-sub".into(),
            sdp: "v=0\r\n".into(),
        }))),
        &rt,
    );

    // Request remote video -> first TrackSubscribe emitted immediately.
    handle
        .set_video_interest(vec![VideoTarget {
            participant: remote,
            kind: VideoKind::Camera,
        }])
        .unwrap();
    assert!(matches!(
        drain_one(&calls, gen1, &mut rx).await,
        Command::TrackSubscribe { .. }
    ));

    // Post-join voice-rate-limited error is non-fatal and triggers backoff retry.
    let rejection = Event::Known(KnownEvent::Error(Box::new(ErrorEvent {
        message: "rate limited".into(),
        code: Some("voice-rate-limited".into()),
        channel_id: Some(ch.to_string()),
        peer_id: Some("peer-sub".into()),
    })));
    for (before_retry_ms, remaining_ms) in [(100, 150), (300, 200)] {
        calls.on_gateway_event(gen1, &rejection, &rt);
        assert_eq!(handle.snapshot().phase, CallPhase::Joined);
        tokio::time::advance(Duration::from_millis(before_retry_ms)).await;
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_none());
        tokio::time::advance(Duration::from_millis(remaining_ms)).await;
        assert!(matches!(
            drain_one(&calls, gen1, &mut rx).await,
            Command::TrackSubscribe { .. }
        ));
    }
}
