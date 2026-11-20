//! Loopback integration tests for session-owned call control, client lease and candidate flood fairness.
#![cfg(feature = "test-support")]

use std::{error::Error, sync::Arc, time::Duration};

use papo_api::wire::{
    Channel, ChannelPermissionEntry, ChannelPermissions, LoginRequest, RegisterRequest, Role,
    RolePermissions, UserProfile,
};
use papo_sdk::{
    AuthService, Backoff, CallConfig, CallEndReason, CallError, CallPhase, Client, ClientConfig,
    ConnectionState, DataRoot, FakeMediaFactory, FakeVideoDestination, Jitter, MediaReportKind,
    MediaTransportState, Session, UnixMicros, Uuid, VideoCodec, VideoDestinationId, VideoKind,
    VideoTarget, secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, TestServer, VoiceFault};
use tokio::runtime::Runtime;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

async fn register(auth: &AuthService, username: &str) -> Result<papo_api::wire::RegisterResponse> {
    Ok(auth
        .register(RegisterRequest {
            username: username.to_owned(),
            password: "Aabcdefghijklmnop!".to_owned(),
        })
        .await?)
}

async fn login(auth: &AuthService, username: &str) -> Result<Session> {
    Ok(auth
        .login(
            LoginRequest {
                username: username.to_owned(),
                password: "Aabcdefghijklmnop!".to_owned(),
            },
            UnixMicros(0),
        )
        .await?)
}

async fn online(session: &Session) -> Result<papo_sdk::Generation> {
    let mut rx = session.subscribe_connection();
    loop {
        let status = *rx.borrow_and_update();
        if status.state == ConnectionState::Online {
            return Ok(status.generation);
        }
        rx.changed().await?;
    }
}

struct NoJitter;
impl Jitter for NoJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}

struct CallHarness {
    client: Option<Client>,
    server: Option<TestServer>,
    server_rt: Option<Runtime>,
    _profile: tempfile::TempDir,
    media: Option<FakeMediaFactory>,
}

impl CallHarness {
    fn open(media: Option<FakeMediaFactory>, call_config: CallConfig) -> Result<Self> {
        let server_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let server = server_rt.block_on(TestServer::start(Config::default()))?;
        let profile = tempfile::tempdir()?;
        let store = Arc::new(ScriptedSecretStore::default());
        let config = ClientConfig {
            insecure_loopback: true,
            jitter: Arc::new(NoJitter),
            backoff: Backoff {
                initial: Duration::from_millis(50),
                maximum: Duration::from_millis(100),
            },
            call: call_config,
            media_engine: media
                .as_ref()
                .map(|m| Arc::new(m.clone()) as Arc<dyn papo_sdk::MediaEngineFactory>),
            ..ClientConfig::default()
        };
        let client = Client::open(DataRoot::new(profile.path()), store, config)?;
        Ok(Self {
            client: Some(client),
            server: Some(server),
            server_rt: Some(server_rt),
            _profile: profile,
            media,
        })
    }

    fn client(&self) -> &Client {
        self.client.as_ref().expect("client open")
    }

    fn server(&self) -> &TestServer {
        self.server.as_ref().expect("server open")
    }

    fn media(&self) -> &FakeMediaFactory {
        self.media.as_ref().expect("media factory configured")
    }

    fn run<F: std::future::Future>(&self, work: F) -> Result<F::Output> {
        Ok(self
            .client()
            .block_on(async { tokio::time::timeout(Duration::from_secs(20), work).await })??)
    }

    fn finish(mut self) -> Result {
        if let Some(mut client) = self.client.take() {
            let report = client.shutdown()?;
            assert!(report.reads_drained);
        }
        if let (Some(server), Some(rt)) = (self.server.take(), self.server_rt.as_ref()) {
            rt.block_on(server.shutdown())?;
        }
        if let Some(rt) = self.server_rt.take() {
            rt.shutdown_timeout(Duration::from_secs(3));
        }
        Ok(())
    }
}

async fn wait_for_channel(session: &Session, channel: Uuid) -> Result {
    let mut resources = session.subscribe_resources();
    loop {
        if resources.borrow().channels.get(channel).is_some() {
            return Ok(());
        }
        resources.changed().await?;
    }
}

async fn wait_phase(
    handle: &papo_sdk::CallHandle,
    expected: impl Fn(CallPhase) -> bool,
) -> Result<papo_sdk::CallSnapshot> {
    let mut rx = handle.subscribe();
    loop {
        let snap = rx.borrow_and_update().clone();
        if expected(snap.phase) {
            return Ok(snap);
        }
        rx.changed().await?;
    }
}

#[test]
fn join_reattach_back_to_back_rejoin_and_delayed_old_leave() -> Result {
    let h = CallHarness::open(Some(FakeMediaFactory::new()), CallConfig::default())?;
    h.run(async {
        let client = h.client().handle();
        let server_id = client
            .add_server(&h.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = client.auth(server_id)?;
        register(&auth, "voice-alice").await?;
        let session = login(&auth, "voice-alice").await?;
        let gen1 = online(&session).await?;

        let ch_a: Uuid = h
            .server()
            .create_voice_channel("alpha")
            .await?
            .id
            .ok_or("ch_a")?
            .parse()?;
        let ch_b: Uuid = h
            .server()
            .create_voice_channel("beta")
            .await?
            .id
            .ok_or("ch_b")?
            .parse()?;
        wait_for_channel(&session, ch_a).await?;
        wait_for_channel(&session, ch_b).await?;

        // Join A and verify reattach via Calls::active().
        let call_a = session.calls().join(ch_a)?;
        let snap_a = wait_phase(&call_a, |p| p == CallPhase::Joined).await?;
        assert_eq!(snap_a.video_codec, Some(VideoCodec::H264));
        assert!(snap_a.settings.muted);
        assert!(!snap_a.settings.camera_on);
        let reattached = session.calls().active().ok_or("active")?;
        assert_eq!(reattached.attempt(), call_a.attempt());
        assert_eq!(
            client.active_call().map(|(acc, _)| acc),
            Some(session.account())
        );

        // Arm delayed voice_leave so A's self-leave arrives AFTER B's voice_joined.
        h.server()
            .push_voice_fault(VoiceFault::DelayNextVoiceLeave)
            .await;

        // Back-to-back A -> B on the SAME socket generation (no socket rotation!).
        let call_b = session.calls().join(ch_b)?;
        let snap_b = wait_phase(&call_b, |p| p == CallPhase::Joined).await?;
        assert_eq!(snap_b.channel, ch_b);
        assert_eq!(snap_b.generation, gen1);
        assert_ne!(snap_a.peer_id, snap_b.peer_id);
        assert_eq!(
            call_a.snapshot().phase,
            CallPhase::Ended(CallEndReason::Switched)
        );

        // Flush A's delayed self-leave now: it carries A's old peer_id and MUST NOT end B!
        h.server().flush_delayed_voice_leaves().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(call_b.snapshot().phase, CallPhase::Joined);

        // Back-to-back B -> B on same channel with delayed leave too.
        h.server()
            .push_voice_fault(VoiceFault::DelayNextVoiceLeave)
            .await;
        let call_b2 = session.calls().join(ch_b)?;
        let snap_b2 = wait_phase(&call_b2, |p| p == CallPhase::Joined).await?;
        assert_eq!(snap_b2.generation, gen1);
        assert_ne!(snap_b.peer_id, snap_b2.peer_id);
        h.server().flush_delayed_voice_leaves().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(call_b2.snapshot().phase, CallPhase::Joined);

        // Seed remote publisher in B, negotiate offer->answer, subscribe and bind video.
        assert_remote_video_bind_and_leave(&h, &call_b2, ch_b).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    h.finish()
}

async fn assert_remote_video_bind_and_leave(
    h: &CallHarness,
    call: &papo_sdk::CallHandle,
    ch_b: Uuid,
) -> Result {
    let bob = Uuid::from_u128(777);
    h.server()
        .seed_voice_occupant(&ch_b.to_string(), &bob.to_string(), false, true, false)
        .await;
    h.server()
        .emit_voice_event(
            None,
            papo_api::gateway::KnownEvent::VoiceStateUpdate(Box::new(
                papo_api::gateway::VoiceStateUpdateEvent {
                    channel_id: ch_b.to_string(),
                    user_id: bob.to_string(),
                    muted: false,
                    camera_on: true,
                    screen_sharing: false,
                },
            )),
        )
        .await?;
    let lease = h.media().latest_lease().ok_or("lease")?;
    lease.emit(MediaReportKind::LocalOffer {
        sdp: "v=0\r\n".into(),
    });
    lease.emit(MediaReportKind::TransportState(
        MediaTransportState::Connected,
    ));
    wait_phase(call, |p| p == CallPhase::Connected).await?;

    let dest = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(42)));
    call.bind_video(bob, VideoKind::Camera, dest.clone())?;
    call.set_video_interest(vec![VideoTarget {
        participant: bob,
        kind: VideoKind::Camera,
    }])?;
    let mut sub = call.subscribe();
    loop {
        let snap = sub.borrow_and_update().clone();
        if snap.video_bindings.iter().any(|b| {
            b.participant == bob
                && b.kind == VideoKind::Camera
                && b.destination_id == Some(VideoDestinationId::from_raw(42))
        }) {
            break;
        }
        sub.changed().await?;
    }

    call.leave();
    assert!(dest.is_revoked());
    Ok(())
}

#[test]
fn cross_account_switch_busy_lease_and_viewed_account_independence() -> Result {
    let media = FakeMediaFactory::new();
    let h = CallHarness::open(Some(media.clone()), CallConfig::default())?;
    h.run(async {
        let client = h.client().handle();
        let server_id = client
            .add_server(&h.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = client.auth(server_id)?;
        register(&auth, "acct-one").await?;
        let s1 = login(&auth, "acct-one").await?;
        online(&s1).await?;

        register(&auth, "acct-two").await?;
        let s2 = login(&auth, "acct-two").await?;
        online(&s2).await?;

        let ch: Uuid = h
            .server()
            .create_voice_channel("shared-room")
            .await?
            .id
            .ok_or("ch")?
            .parse()?;
        wait_for_channel(&s1, ch).await?;
        wait_for_channel(&s2, ch).await?;

        let call1 = s1.calls().join(ch)?;
        wait_phase(&call1, |p| p == CallPhase::Joined).await?;

        // Switching the viewed active account does NOT transfer or end the call.
        client
            .set_active_account(server_id, Some(s2.account()))
            .await?;
        assert_eq!(call1.snapshot().phase, CallPhase::Joined);

        // Direct join on account 2 is rejected with Busy while account 1 holds the lease.
        assert!(matches!(s2.calls().join(ch), Err(CallError::Busy)));

        // Hold account 1's drain briefly to prove switch_call waits for drain before starting s2.
        let lease1 = media.latest_lease().ok_or("lease1")?;
        lease1.set_auto_drain(false);
        let switch_client = client.clone();
        let s2_account = s2.account();
        let switch_task =
            tokio::spawn(async move { switch_client.switch_call(s2_account, ch).await });
        while !lease1.is_stopped() {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            call1.snapshot().phase,
            CallPhase::Ended(CallEndReason::Switched)
        );
        // Account 2 has not started its media lease yet while lease1 has not drained.
        assert_eq!(media.start_count(), 1);
        lease1.emit(MediaReportKind::Drained);

        let call2 = switch_task.await??;
        let snap2 = wait_phase(&call2, |p| p == CallPhase::Joined).await?;
        assert_eq!(snap2.account, s2.account());
        assert_eq!(media.start_count(), 2);
        call2.leave();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    h.finish()
}

#[test]
fn reconnect_logout_and_removal_end_calls_without_autojoin() -> Result {
    let h = CallHarness::open(Some(FakeMediaFactory::new()), CallConfig::default())?;
    h.run(async {
        let client = h.client().handle();
        let server_id = client
            .add_server(&h.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = client.auth(server_id)?;
        register(&auth, "lifecycle-user").await?;
        let session = login(&auth, "lifecycle-user").await?;
        let gen1 = online(&session).await?;
        let ch: Uuid = h
            .server()
            .create_voice_channel("lounge")
            .await?
            .id
            .ok_or("ch")?
            .parse()?;
        wait_for_channel(&session, ch).await?;

        // 1. Gateway drop ends call with ConnectionLost and reconnect never auto-rejoins.
        let call1 = session.calls().join(ch)?;
        wait_phase(&call1, |p| p == CallPhase::Joined).await?;
        h.server().disconnect_gateways();
        wait_phase(&call1, |p| {
            p == CallPhase::Ended(CallEndReason::ConnectionLost)
        })
        .await?;
        let mut conn = session.subscribe_connection();
        loop {
            let status = *conn.borrow_and_update();
            if status.state == papo_sdk::ConnectionState::Online && status.generation != gen1 {
                break;
            }
            conn.changed().await?;
        }
        assert!(session.calls().active().is_none());

        // 2. Logout ends active call with LoggedOut.
        let call2 = session.calls().join(ch)?;
        wait_phase(&call2, |p| p == CallPhase::Joined).await?;
        session.auth().logout().await?;
        assert_eq!(
            call2.snapshot().phase,
            CallPhase::Ended(CallEndReason::LoggedOut)
        );

        // 3. Account removal ends active call with AccountRemoved.
        let session2 = login(&auth, "lifecycle-user").await?;
        online(&session2).await?;
        wait_for_channel(&session2, ch).await?;
        let call3 = session2.calls().join(ch)?;
        wait_phase(&call3, |p| p == CallPhase::Joined).await?;
        client.remove_account(session2.account()).await?;
        assert_eq!(
            call3.snapshot().phase,
            CallPhase::Ended(CallEndReason::AccountRemoved)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    h.finish()
}

#[test]
fn chat_and_heartbeat_keep_flowing_under_candidate_flood() -> Result {
    let media = FakeMediaFactory::new();
    let h = CallHarness::open(Some(media.clone()), CallConfig::default())?;
    h.run(async {
        let client = h.client().handle();
        let server_id = client
            .add_server(&h.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = client.auth(server_id)?;
        register(&auth, "flood-user").await?;
        let session = login(&auth, "flood-user").await?;
        online(&session).await?;

        let text_ch: Uuid = h
            .server()
            .create_channel("chat")
            .await?
            .id
            .ok_or("text_ch")?
            .parse()?;
        let voice_ch: Uuid = h
            .server()
            .create_voice_channel("voice-flood")
            .await?
            .id
            .ok_or("voice_ch")?
            .parse()?;
        wait_for_channel(&session, text_ch).await?;
        wait_for_channel(&session, voice_ch).await?;

        let mut events = session.subscribe_events();
        let call = session.calls().join(voice_ch)?;
        wait_phase(&call, |p| p == CallPhase::Joined).await?;
        let lease = media.latest_lease().ok_or("lease")?;

        // Flood 120 ICE candidates while emitting chat messages on the same gateway.
        for i in 0..120_u32 {
            lease.emit(MediaReportKind::LocalIceCandidate {
                candidate: format!(
                    "candidate:{i} 1 UDP 2122260223 127.0.0.1 {} typ host",
                    50000 + i
                ),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            });
            if i % 30 == 0 {
                let msg_id = Uuid::from_u128(9000 + u128::from(i)).to_string();
                h.server()
                    .emit_chat_message(
                        &text_ch.to_string(),
                        &msg_id,
                        &session.account().user().to_string(),
                    )
                    .await;
            }
        }

        // Verify all 4 chat messages arrive promptly through session.events and voice events never pollute it.
        let mut seen_messages = 0_usize;
        while seen_messages < 4 {
            let event = events.next().await?;
            assert_ne!(event.value.kind(), "voice_joined");
            assert_ne!(event.value.kind(), "voice_ice_candidate");
            if event.value.kind() == "message" {
                seen_messages += 1;
            }
        }

        // Wait until server observed the flooded ICE candidates and connection remained Online.
        loop {
            let count = h
                .server()
                .recorded_voice_commands()
                .await
                .iter()
                .filter(|cmd| cmd.kind == "voice_ice_candidate")
                .count();
            if count >= 120 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            session.subscribe_connection().borrow().state,
            papo_sdk::ConnectionState::Online
        );
        call.leave();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    h.finish()
}

async fn seed_scoped_voice_channels(
    server: &TestServer,
    role_id: Uuid,
    allowed_ch: Uuid,
    denied_ch: Uuid,
    occupant: Uuid,
) -> Result {
    server
        .set_roles(vec![Role {
            id: Some(role_id.to_string()),
            name: Some("voice-only".into()),
            permissions: Some(RolePermissions::default()),
            ..Role::default()
        }])
        .await;
    server
        .set_channel(Channel {
            id: Some(allowed_ch.to_string()),
            name: Some("allowed".into()),
            r#type: Some("voice".into()),
            position: Some(1),
            permissions: Some(vec![ChannelPermissionEntry {
                role_id: Some(role_id.to_string()),
                role_name: None,
                permissions: Some(ChannelPermissions {
                    read_channel: Some(false),
                    send_messages: Some(false),
                    delete_messages: Some(false),
                    connect_voice: Some(true),
                }),
            }]),
            ..Channel::default()
        })
        .await?;
    server
        .set_channel(Channel {
            id: Some(denied_ch.to_string()),
            name: Some("denied".into()),
            r#type: Some("voice".into()),
            position: Some(2),
            permissions: Some(vec![ChannelPermissionEntry {
                role_id: Some(role_id.to_string()),
                role_name: None,
                permissions: Some(ChannelPermissions {
                    read_channel: Some(true),
                    send_messages: Some(true),
                    delete_messages: Some(false),
                    connect_voice: Some(false),
                }),
            }]),
            ..Channel::default()
        })
        .await?;
    server
        .seed_voice_occupant(
            &allowed_ch.to_string(),
            &occupant.to_string(),
            false,
            true,
            false,
        )
        .await;
    server
        .seed_voice_occupant(
            &denied_ch.to_string(),
            &occupant.to_string(),
            false,
            true,
            false,
        )
        .await;
    Ok(())
}

#[test]
fn missing_factory_still_watches_admitted_cold_presence_and_filters_denied_channels() -> Result {
    let h = CallHarness::open(None, CallConfig::default())?;
    h.run(async {
        let client = h.client().handle();
        let server_id = client
            .add_server(&h.server().base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = client.auth(server_id)?;
        register(&auth, "watcher").await?;

        let role_id = Uuid::from_u128(501);
        let allowed_ch = Uuid::from_u128(502);
        let denied_ch = Uuid::from_u128(503);
        let occupant = Uuid::from_u128(504);

        seed_scoped_voice_channels(h.server(), role_id, allowed_ch, denied_ch, occupant).await?;

        let session = login(&auth, "watcher").await?;
        h.server()
            .set_profile(UserProfile {
                id: Some(session.account().user().to_string()),
                username: Some("watcher".into()),
                roles: Some(vec![papo_api::wire::RoleSummary {
                    id: Some(role_id.to_string()),
                    name: Some("voice-only".into()),
                    color: None,
                }]),
                ..UserProfile::default()
            })
            .await?;
        online(&session).await?;

        // Cold user_voice for allowed_ch appears with device: None; denied_ch is filtered out.
        let mut rooms_rx = session.calls().subscribe_rooms();
        loop {
            let snap = rooms_rx.borrow_and_update().clone();
            if snap.rooms.contains_key(&allowed_ch) {
                assert!(!snap.rooms.contains_key(&denied_ch));
                assert_eq!(snap.rooms[&allowed_ch].members.len(), 1);
                assert_eq!(snap.rooms[&allowed_ch].members[0].user, occupant);
                assert!(snap.rooms[&allowed_ch].members[0].device.is_none());
                break;
            }
            rooms_rx.changed().await?;
        }
        assert!(matches!(
            session.calls().join(allowed_ch),
            Err(CallError::MediaUnavailable)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    h.finish()
}
