//! Real 14-0 Go backend + PostgreSQL voice proof for Session 14a.
//! Run only through `scripts/papo-backend-proof.py`, never against a live user server.
#![cfg(feature = "test-support")]

use std::{error::Error, sync::Arc, time::Duration};

use papo_api::{
    RestClient,
    gateway::{Command, Event, Gateway, KnownEvent},
    wire::{
        ChannelPermissions, CreateChannelRequest, CreateRoleRequest, LoginRequest, RegisterRequest,
        UpdateChannelPermissionsRequest,
    },
};
use papo_sdk::{
    CallConfig, CallEndReason, CallError, CallHandle, CallPhase, CallSnapshot, Client,
    ClientConfig, ConnectionState, DataRoot, FakeMediaFactory, MediaReportKind, Session,
    UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

async fn next_known(
    gw: &mut Gateway,
    timeout_secs: u64,
    mut pred: impl FnMut(&KnownEvent) -> bool,
) -> Result<KnownEvent> {
    tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        loop {
            let Some(event) = gw.next().await? else {
                return Err::<KnownEvent, Box<dyn Error>>("gateway closed early".into());
            };
            if let Event::Known(known) = event
                && pred(&known)
            {
                return Ok(known);
            }
        }
    })
    .await?
}

async fn login_sdk(
    client: &Client,
    server: papo_sdk::ServerId,
    password: &str,
    username: &str,
    register: bool,
) -> Result<Session> {
    let handle = client.handle();
    let auth = handle.auth(server)?;
    if register {
        auth.register(RegisterRequest {
            username: username.into(),
            password: password.into(),
        })
        .await?;
    }
    let session = auth
        .login(
            LoginRequest {
                username: username.into(),
                password: password.into(),
            },
            UnixMicros(0),
        )
        .await?;
    let mut connection = session.subscribe_connection();
    loop {
        let status = *connection.borrow_and_update();
        if status.generation == session.generation() && status.state == ConnectionState::Online {
            return Ok(session);
        }
        connection.changed().await?;
    }
}

async fn login_raw(address: &str, username: &str, password: &str) -> Result<RestClient> {
    let rest = RestClient::loopback(address)?;
    rest.login(&LoginRequest {
        username: username.into(),
        password: password.into(),
    })
    .await?;
    Ok(rest)
}

async fn wait_phase(call: &CallHandle, pred: impl Fn(CallPhase) -> bool) -> Result<CallSnapshot> {
    let mut rx = call.subscribe();
    loop {
        let snap = rx.borrow_and_update().clone();
        if pred(snap.phase) {
            return Ok(snap);
        }
        rx.changed().await?;
    }
}

async fn wait_channel(session: &Session, channel: Uuid) -> Result {
    let mut rx = session.subscribe_resources();
    loop {
        if rx.borrow_and_update().channels.get(channel).is_some() {
            return Ok(());
        }
        rx.changed().await?;
    }
}

async fn wait_role_assigned(session: &Session, role_id: Uuid) -> Result {
    let mut rx = session.subscribe_resources();
    let user = session.account().user();
    loop {
        let snap = rx.borrow_and_update().clone();
        if snap
            .users
            .get(user)
            .and_then(|u| u.roles.as_ref())
            .is_some_and(|roles| roles.contains(&role_id))
        {
            return Ok(());
        }
        rx.changed().await?;
    }
}

#[test]
#[ignore = "requires isolated pinned Go/PostgreSQL; run scripts/papo-backend-proof.py"]
fn real_14_0_backend_proves_owner_binding_rejoin_audience_scope_and_typed_rejections() -> Result {
    let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS")?;
    let password = std::env::var("PAPO_PROOF_PASSWORD")?;
    let directory = tempfile::tempdir()?;
    let media = FakeMediaFactory::new();
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            call: CallConfig::default(),
            media_engine: Some(Arc::new(media.clone())),
            ..ClientConfig::default()
        },
    )?;

    client.block_on(async {
        tokio::time::timeout(Duration::from_secs(90), async {
            let server = client
                .handle()
                .add_server(&address, UnixMicros(0))
                .await?
                .id;
            let owner = login_sdk(&client, server, &password, "cursorproof", false).await?;
            let voiceonly = login_sdk(&client, server, &password, "voiceonlyproof", true).await?;
            let readonly = login_sdk(&client, server, &password, "readonlyproof", true).await?;

            // 1. GET /voice/ice-servers requires authentication.
            let unauth = RestClient::loopback(&address)?;
            assert!(unauth.ice_servers().await.is_err());
            let owner_raw = login_raw(&address, "cursorproof", &password).await?;
            let _ice = owner_raw.ice_servers().await?;

            // Create two open voice channels, one text channel, and one role-scoped voice channel.
            let ops = owner.operations();
            let text_ch: Uuid = ops
                .create_channel(CreateChannelRequest {
                    name: "voice-proof-text".into(),
                    r#type: Some("text".into()),
                    ..Default::default()
                })
                .await?
                .value
                .id
                .ok_or("text_ch")?
                .parse()?;
            let voice_a: Uuid = ops
                .create_channel(CreateChannelRequest {
                    name: "voice-proof-a".into(),
                    r#type: Some("voice".into()),
                    ..Default::default()
                })
                .await?
                .value
                .id
                .ok_or("voice_a")?
                .parse()?;
            let voice_b: Uuid = ops
                .create_channel(CreateChannelRequest {
                    name: "voice-proof-b".into(),
                    r#type: Some("voice".into()),
                    ..Default::default()
                })
                .await?
                .value
                .id
                .ok_or("voice_b")?
                .parse()?;
            wait_channel(&owner, voice_a).await?;
            wait_channel(&owner, voice_b).await?;

            // 2. Owner-client binding + typed rejections on raw sockets.
            prove_owner_binding_and_typed_rejections(&address, &password, text_ch, voice_a).await?;

            // 3. SDK back-to-back rejoin (A -> A and A -> B) on the same socket generation,
            // plus verifying late voice_leave carries old peer_id and invalid SDP ends the call.
            prove_sdk_rejoin_and_invalid_sdp(&address, &password, &owner, &media, voice_a, voice_b)
                .await?;

            // 4. Audience scope (connect_voice without read_channel vs read_channel without
            // connect_voice) and self-leave delivery even after connect_voice is revoked.
            prove_audience_scope_and_self_leave(&address, &password, &owner, &voiceonly, &readonly)
                .await?;

            Ok::<_, Box<dyn Error>>(())
        })
        .await?
    })??;

    let report = client.shutdown()?;
    assert!(report.state.finished && report.shared.finished);
    Ok(())
}

async fn prove_owner_binding_and_typed_rejections(
    address: &str,
    password: &str,
    text_ch: Uuid,
    voice_a: Uuid,
) -> Result {
    let raw1 = login_raw(address, "cursorproof", password).await?;
    let raw2 = login_raw(address, "cursorproof", password).await?;
    let mut gw1 = Gateway::connect(&raw1).await?;
    let mut gw2 = Gateway::connect(&raw2).await?;
    // Drain initial presence_sync on both sockets.
    let _ = next_known(&mut gw1, 5, |e| matches!(e, KnownEvent::PresenceSync(_))).await?;
    let _ = next_known(&mut gw2, 5, |e| matches!(e, KnownEvent::PresenceSync(_))).await?;

    // Joining a text channel returns typed voice-forbidden.
    gw1.send(&Command::VoiceJoin {
        channel_id: text_ch.to_string(),
    })
    .await?;
    let err = next_known(&mut gw1, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-forbidden"));

    // Leaving when not in a room returns typed voice-not-found.
    gw1.send(&Command::VoiceLeave {
        channel_id: voice_a.to_string(),
    })
    .await?;
    let err = next_known(&mut gw1, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-not-found"));

    // Socket 1 joins voice_a and becomes owner of the peer.
    gw1.send(&Command::VoiceJoin {
        channel_id: voice_a.to_string(),
    })
    .await?;
    let joined = next_known(&mut gw1, 5, |e| matches!(e, KnownEvent::VoiceJoined(_))).await?;
    let KnownEvent::VoiceJoined(joined) = joined else {
        unreachable!()
    };
    let peer1 = joined.peer_id.clone();
    assert_ne!(peer1, "");
    assert_ne!(joined.video_codec, "");

    // Second join while already in room returns typed voice-already-in-room.
    gw1.send(&Command::VoiceJoin {
        channel_id: voice_a.to_string(),
    })
    .await?;
    let err = next_known(&mut gw1, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-already-in-room"));

    // Non-owner socket 2 (same user!) is rejected with voice-not-found for mute and leave.
    gw2.send(&Command::VoiceMute {
        channel_id: voice_a.to_string(),
        muted: false,
    })
    .await?;
    let err = next_known(&mut gw2, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-not-found"));

    gw2.send(&Command::VoiceLeave {
        channel_id: voice_a.to_string(),
    })
    .await?;
    let err = next_known(&mut gw2, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-not-found"));

    // Closing owner socket 1 while socket 2 stays open removes the peer and emits voice_leave
    // carrying peer1's exact peer_id to socket 2.
    gw1.close().await?;
    drop(gw1);
    let leave = next_known(
        &mut gw2,
        5,
        |e| matches!(e, KnownEvent::VoiceLeave(ev) if ev.channel_id == voice_a.to_string()),
    )
    .await?;
    let KnownEvent::VoiceLeave(leave) = leave else {
        unreachable!()
    };
    assert_eq!(leave.peer_id, peer1);
    gw2.close().await?;
    Ok(())
}

async fn prove_sdk_rejoin_and_invalid_sdp(
    address: &str,
    password: &str,
    owner: &Session,
    media: &FakeMediaFactory,
    voice_a: Uuid,
    voice_b: Uuid,
) -> Result {
    // Open an observer socket to watch peer_ids on voice_leave broadcasts.
    let watcher_raw = login_raw(address, "cursorproof", password).await?;
    let mut watcher = Gateway::connect(&watcher_raw).await?;
    let _ = next_known(&mut watcher, 5, |e| {
        matches!(e, KnownEvent::PresenceSync(_))
    })
    .await?;

    let call1 = owner.calls().join(voice_a)?;
    let snap1 = wait_phase(&call1, |p| p == CallPhase::Joined).await?;
    let peer1 = snap1.peer_id.clone().ok_or("peer1")?;
    let gen1 = snap1.generation;
    assert!(snap1.video_codec.is_some());

    // Back-to-back A -> A rejoin on the SAME socket generation (no barrier, no socket rotation).
    let call2 = owner.calls().join(voice_a)?;
    assert_eq!(
        call1.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    let snap2 = wait_phase(&call2, |p| p == CallPhase::Joined).await?;
    let peer2 = snap2.peer_id.clone().ok_or("peer2")?;
    assert_eq!(snap2.generation, gen1);
    assert_ne!(peer2, peer1);
    // Observer confirms A's leave carried peer1.
    let leave1 = next_known(
        &mut watcher,
        5,
        |e| matches!(e, KnownEvent::VoiceLeave(ev) if ev.peer_id == peer1),
    )
    .await?;
    assert!(matches!(leave1, KnownEvent::VoiceLeave(_)));

    // Back-to-back A -> B rejoin on the SAME socket generation.
    let call3 = owner.calls().join(voice_b)?;
    assert_eq!(
        call2.snapshot().phase,
        CallPhase::Ended(CallEndReason::Switched)
    );
    let snap3 = wait_phase(&call3, |p| p == CallPhase::Joined).await?;
    let peer3 = snap3.peer_id.clone().ok_or("peer3")?;
    assert_eq!(snap3.generation, gen1);
    assert_ne!(peer3, peer2);
    // Observer confirms A's leave carried peer2 while B remains Joined.
    let _ = next_known(
        &mut watcher,
        5,
        |e| matches!(e, KnownEvent::VoiceLeave(ev) if ev.peer_id == peer2),
    )
    .await?;
    assert_eq!(call3.snapshot().phase, CallPhase::Joined);

    // Emitting malformed SDP triggers real backend voice-invalid-sdp and ends the SDK call cleanly.
    let active_media = media.latest_lease().ok_or("active_media")?;
    active_media.emit(MediaReportKind::LocalOffer {
        sdp: "not-a-valid-sdp".into(),
    });
    let ended = wait_phase(&call3, |p| p == CallPhase::Ended(CallEndReason::InvalidSdp)).await?;
    assert_eq!(ended.phase, CallPhase::Ended(CallEndReason::InvalidSdp));

    // Clean leave broadcast for peer3 finishes on the backend.
    let _ = next_known(
        &mut watcher,
        5,
        |e| matches!(e, KnownEvent::VoiceLeave(ev) if ev.peer_id == peer3),
    )
    .await?;
    watcher.close().await?;
    Ok(())
}

async fn setup_scoped_voice_channel(
    owner: &Session,
    voiceonly: &Session,
    readonly: &Session,
) -> Result<(Uuid, Uuid)> {
    let ops = owner.operations();
    let voice_role: Uuid = ops
        .create_role(CreateRoleRequest {
            name: "proof-voice-only".into(),
            users: Some(vec![voiceonly.account().user().to_string()]),
            ..Default::default()
        })
        .await?
        .value
        .id
        .ok_or("voice_role")?
        .parse()?;
    let read_role: Uuid = ops
        .create_role(CreateRoleRequest {
            name: "proof-read-only".into(),
            users: Some(vec![readonly.account().user().to_string()]),
            ..Default::default()
        })
        .await?
        .value
        .id
        .ok_or("read_role")?
        .parse()?;

    let scoped_ch: Uuid = ops
        .create_channel(CreateChannelRequest {
            name: "voice-proof-scoped".into(),
            r#type: Some("voice".into()),
            access_roles: Some(vec![voice_role.to_string(), read_role.to_string()]),
            ..Default::default()
        })
        .await?
        .value
        .id
        .ok_or("scoped_ch")?
        .parse()?;
    ops.update_channel_permissions(
        scoped_ch,
        voice_role,
        UpdateChannelPermissionsRequest {
            permissions: ChannelPermissions {
                read_channel: Some(false),
                send_messages: Some(false),
                delete_messages: Some(false),
                connect_voice: Some(true),
            },
        },
    )
    .await?;
    ops.update_channel_permissions(
        scoped_ch,
        read_role,
        UpdateChannelPermissionsRequest {
            permissions: ChannelPermissions {
                read_channel: Some(true),
                send_messages: Some(true),
                delete_messages: Some(false),
                connect_voice: Some(false),
            },
        },
    )
    .await?;

    // Refresh both accounts so they see the final channel overwrites and role assignments.
    wait_role_assigned(voiceonly, voice_role).await?;
    wait_role_assigned(readonly, read_role).await?;
    voiceonly.reconnect()?;
    readonly.reconnect()?;
    for s in [voiceonly, readonly] {
        let mut conn = s.subscribe_connection();
        loop {
            let st = *conn.borrow_and_update();
            if st.generation == s.generation() && st.state == ConnectionState::Online {
                break;
            }
            conn.changed().await?;
        }
        wait_channel(s, scoped_ch).await?;
    }
    Ok((scoped_ch, voice_role))
}

async fn prove_audience_scope_and_self_leave(
    address: &str,
    password: &str,
    owner: &Session,
    voiceonly: &Session,
    readonly: &Session,
) -> Result {
    let (scoped_ch, voice_role) = setup_scoped_voice_channel(owner, voiceonly, readonly).await?;

    // Read-only user is forbidden on SDK AND rejected with voice-forbidden on raw WS.
    assert!(matches!(
        readonly.calls().join(scoped_ch),
        Err(CallError::Forbidden)
    ));
    let ro_raw = login_raw(address, "readonlyproof", password).await?;
    let mut ro_gw = Gateway::connect(&ro_raw).await?;
    let _ = next_known(&mut ro_gw, 5, |e| matches!(e, KnownEvent::PresenceSync(_))).await?;
    ro_gw
        .send(&Command::VoiceJoin {
            channel_id: scoped_ch.to_string(),
        })
        .await?;
    let err = next_known(&mut ro_gw, 5, |e| matches!(e, KnownEvent::Error(_))).await?;
    let KnownEvent::Error(err) = err else {
        unreachable!()
    };
    assert_eq!(err.code.as_deref(), Some("voice-forbidden"));

    // Voice-only user (read_channel=false, connect_voice=true) joins successfully via SDK.
    let call = voiceonly.calls().join(scoped_ch)?;
    let joined = wait_phase(&call, |p| p == CallPhase::Joined).await?;
    assert_eq!(joined.channel, scoped_ch);
    call.set_muted(false)?;

    // Read-only user must NOT receive voice broadcasts for scoped_ch.
    let leaked = tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            let Some(event) = ro_gw.next().await? else {
                return Ok::<bool, Box<dyn Error>>(false);
            };
            if let Event::Known(KnownEvent::VoiceStateUpdate(ev)) = event
                && ev.channel_id == scoped_ch.to_string()
            {
                return Ok(true);
            }
        }
    })
    .await;
    assert!(
        leaked.is_err(),
        "readonly user received voice broadcast without connect_voice"
    );
    call.leave();
    let _ = wait_phase(&call, |p| p == CallPhase::Ended(CallEndReason::LocalLeave)).await?;
    ro_gw.close().await?;

    prove_self_leave_after_revoke(address, password, owner, voiceonly, scoped_ch, voice_role).await
}

async fn prove_self_leave_after_revoke(
    address: &str,
    password: &str,
    owner: &Session,
    voiceonly: &Session,
    scoped_ch: Uuid,
    voice_role: Uuid,
) -> Result {
    // Self-leave is still delivered to the leaving user even after connect_voice is revoked.
    let vo_raw = login_raw(address, "voiceonlyproof", password).await?;
    let mut vo_gw = Gateway::connect(&vo_raw).await?;
    let _ = next_known(&mut vo_gw, 5, |e| matches!(e, KnownEvent::PresenceSync(_))).await?;
    vo_gw
        .send(&Command::VoiceJoin {
            channel_id: scoped_ch.to_string(),
        })
        .await?;
    let vo_joined = next_known(&mut vo_gw, 5, |e| matches!(e, KnownEvent::VoiceJoined(_))).await?;
    let KnownEvent::VoiceJoined(vo_joined) = vo_joined else {
        unreachable!()
    };
    let vo_peer = vo_joined.peer_id;

    // Revoke voice_role while user is in the room, then leave -> self voice_leave still arrives!
    owner
        .operations()
        .remove_role(voiceonly.account().user(), voice_role)
        .await?;
    vo_gw
        .send(&Command::VoiceLeave {
            channel_id: scoped_ch.to_string(),
        })
        .await?;
    let self_leave = next_known(
        &mut vo_gw,
        5,
        |e| matches!(e, KnownEvent::VoiceLeave(ev) if ev.peer_id == vo_peer),
    )
    .await?;
    assert!(matches!(self_leave, KnownEvent::VoiceLeave(_)));
    vo_gw.close().await?;
    Ok(())
}
