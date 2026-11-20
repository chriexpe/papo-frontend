//! Public-API loopback tests for `CallService` in `papo-sdk-ui`.

use std::{error::Error, sync::Arc, time::Duration};

use futures_util::StreamExt;
use papo_sdk::{
    Backoff, CallConfig, CallPhase, Client, ClientConfig, ConnectionState, DataRoot,
    FakeMediaFactory, FakeVideoDestination, Jitter, LoginRequest, MediaReportKind,
    MediaTransportState, RegisterRequest, SpeakingState, UnixMicros, Uuid, VideoDestinationId,
    VideoKind, VideoTarget, secret::test_store::ScriptedSecretStore,
};
use papo_sdk_ui::{CallService, VoiceCapability};
use papo_test_server::{Config, TestServer};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

struct NoJitter;
impl Jitter for NoJitter {
    fn sample(&self, _: Duration) -> Duration {
        Duration::ZERO
    }
}

async fn seed_ui_voice_fixture(
    server: &TestServer,
    role_id: Uuid,
    peer_id: Uuid,
) -> Result<(Uuid, String, Uuid, Uuid)> {
    server.set_voice_role(&role_id.to_string(), "speaker").await;

    let voice_ch_str = server
        .create_voice_channel("stage")
        .await?
        .id
        .ok_or("voice_ch")?;
    let voice_ch: Uuid = voice_ch_str.parse()?;
    // Voice granted WITHOUT read_channel!
    server
        .set_voice_channel_overwrites(&voice_ch_str, vec![(&role_id.to_string(), false, true)])
        .await?;

    let denied_ch_str = server
        .create_voice_channel("vip")
        .await?
        .id
        .ok_or("denied_ch")?;
    let denied_voice_ch: Uuid = denied_ch_str.parse()?;
    server
        .set_voice_channel_overwrites(&denied_ch_str, vec![(&role_id.to_string(), true, false)])
        .await?;

    let text_ch: Uuid = server
        .create_channel("general")
        .await?
        .id
        .ok_or("text_ch")?
        .parse()?;

    server
        .set_voice_member_profile(
            &peer_id.to_string(),
            "ana_user",
            Some("Ana"),
            Some("webp"),
            &[],
        )
        .await?;
    server
        .seed_voice_occupant(&voice_ch_str, &peer_id.to_string(), false, true, false)
        .await;
    Ok((voice_ch, voice_ch_str, denied_voice_ch, text_ch))
}

#[test]
fn call_service_joins_admitted_names_avatars_connect_voice_and_attached_video() -> Result {
    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let server = server_rt.block_on(TestServer::start(Config::default()))?;
    let media = FakeMediaFactory::new();
    let profile = tempfile::tempdir()?;
    let store = Arc::new(ScriptedSecretStore::default());
    let config = ClientConfig {
        insecure_loopback: true,
        jitter: Arc::new(NoJitter),
        backoff: Backoff {
            initial: Duration::from_millis(50),
            maximum: Duration::from_millis(100),
        },
        call: CallConfig::default(),
        media_engine: Some(Arc::new(media.clone())),
        ..ClientConfig::default()
    };
    let mut client = Client::open(DataRoot::new(profile.path()), store, config)?;

    client.block_on(async {
        let handle = client.handle();
        let server_id = handle
            .add_server(&server.base_url(), UnixMicros(0))
            .await?
            .id;
        let auth = handle.auth(server_id)?;
        auth.register(RegisterRequest {
            username: "ui-caller".into(),
            password: "A!abcdefghijklmnop".into(),
        })
        .await?;

        let role_id = Uuid::from_u128(601);
        let peer_id = Uuid::from_u128(605);
        let (voice_ch, voice_ch_str, denied_voice_ch, text_ch) =
            seed_ui_voice_fixture(&server, role_id, peer_id).await?;

        let session = auth
            .login(
                LoginRequest {
                    username: "ui-caller".into(),
                    password: "A!abcdefghijklmnop".into(),
                },
                UnixMicros(0),
            )
            .await?;
        server
            .set_voice_member_profile(
                &session.account().user().to_string(),
                "ui-caller",
                None,
                None,
                &[(&role_id.to_string(), "speaker")],
            )
            .await?;

        let mut conn = session.subscribe_connection();
        while conn.borrow_and_update().state != ConnectionState::Online {
            conn.changed().await?;
        }
        // Load peer profile into admitted resources so avatar_format is available.
        let _ = session.profiles(&[peer_id]).await?;

        let service = CallService::new(session.clone());
        assert_eq!(service.connect_voice(voice_ch), VoiceCapability::Allowed);
        assert_eq!(
            service.connect_voice(denied_voice_ch),
            VoiceCapability::Denied
        );
        assert_eq!(
            service.connect_voice(text_ch),
            VoiceCapability::NotVoiceChannel
        );
        assert_call_service_projection(&server, &media, &service, voice_ch, &voice_ch_str, peer_id)
            .await
    })??;

    client.shutdown()?;
    server_rt.block_on(server.shutdown())?;
    server_rt.shutdown_timeout(Duration::from_secs(3));
    Ok(())
}

async fn assert_call_service_projection(
    server: &TestServer,
    media: &FakeMediaFactory,
    service: &CallService,
    voice_ch: Uuid,
    voice_ch_str: &str,
    peer_id: Uuid,
) -> Result {
    let mut sub = service.subscribe();
    let call = service.join(voice_ch)?;

    let mut call_rx = call.subscribe();
    while call_rx.borrow_and_update().phase != CallPhase::Joined {
        call_rx.changed().await?;
    }
    let lease = media.latest_lease().ok_or("lease")?;
    lease.emit(MediaReportKind::LocalOffer {
        sdp: "v=0\r\n".into(),
    });
    lease.emit(MediaReportKind::TransportState(
        MediaTransportState::Connected,
    ));

    let dest = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(7)));
    call.bind_video(peer_id, VideoKind::Camera, dest)?;
    call.set_video_interest(vec![VideoTarget {
        participant: peer_id,
        kind: VideoKind::Camera,
    }])?;
    server
        .emit_active_speakers(voice_ch_str, &[&peer_id.to_string()])
        .await;

    let view = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = sub.stream.next().await.ok_or("stream closed")?;
            if let Some(active) = &next.active
                && active.phase == CallPhase::Connected
                && let Some(row) = active.participants.iter().find(|p| p.user == peer_id)
                && row.speaking == SpeakingState::Selected
                && !row.video_tracks.is_empty()
            {
                return Ok::<_, Box<dyn Error>>(next);
            }
        }
    })
    .await??;

    let active = view.active.as_ref().ok_or("active")?;
    let ana = active
        .participants
        .iter()
        .find(|p| p.user == peer_id)
        .ok_or("ana row")?;
    assert_eq!(ana.username.as_deref(), Some("ana_user"));
    assert_eq!(ana.nickname.as_deref(), Some("Ana"));
    assert_eq!(ana.avatar_format.as_deref(), Some("webp"));
    assert_eq!(ana.speaking, SpeakingState::Selected);
    assert_eq!(ana.video_tracks.len(), 1);
    assert_eq!(ana.video_tracks[0].kind, VideoKind::Camera);
    assert_eq!(
        ana.video_tracks[0].destination_id,
        Some(VideoDestinationId::from_raw(7))
    );

    call.leave();
    Ok(())
}
