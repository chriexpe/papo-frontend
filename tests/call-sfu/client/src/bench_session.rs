//! Synthetic SFU signaling and media-lease session driver for desktop playback budgets.

use super::{
    FIXTURE_CHANNEL, bench_audio::AudioProbe, bench_profile::ProfileSpec, login, rune_bench::State,
};
use papo_api::gateway::{Command, Event, Gateway, KnownEvent, TrackKind};
use papo_call::{CpuVideoDestination, GstreamerMediaFactory, ReceiveSlots};
use papo_sdk::{
    AudioRoute, CallAttemptId, InitialMediaSettings, MediaCommand, MediaEngineFactory,
    MediaReportKind, MediaReportSink, MediaTransportState, Uuid, VideoCodec, VideoDestinationId,
    VideoKind, VideoRouteFence, VideoSlotBinding, VideoTarget,
};
use rune::AppHandle;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};

pub(super) async fn receive(
    address: &str,
    spec: ProfileSpec,
    audio: Arc<AudioProbe>,
    handle: AppHandle<State>,
    destinations_tx: watch::Sender<Vec<Arc<CpuVideoDestination>>>,
    start_rx: oneshot::Receiver<()>,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    tokio::select! {
        res = start_rx => {
            if res.is_err() { return Ok(()); }
        }
        _ = &mut stop_rx => return Ok(()),
    }

    let targets: Vec<(Uuid, VideoKind)> = spec
        .targets
        .iter()
        .map(|(u, k)| {
            u.parse::<Uuid>()
                .map(|id| (id, *k))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    let destinations: Vec<Arc<CpuVideoDestination>> = targets
        .iter()
        .enumerate()
        .map(|(idx, (participant, kind))| {
            let dest = Arc::new(CpuVideoDestination::new(
                VideoDestinationId::from_raw((idx + 1) as u64),
                VideoTarget {
                    participant: *participant,
                    kind: *kind,
                },
            ));
            let wake_handle = handle.clone();
            dest.set_waker(Some(Arc::new(move || wake_handle.wake())));
            dest
        })
        .collect();
    let _ = destinations_tx.send(destinations.clone());

    let rest = login(address, "fixture", "", false)
        .await
        .map_err(|e| e.to_string())?;
    let mut gateway = Gateway::connect(&rest).await.map_err(|e| e.to_string())?;
    let (sink, mut reports) = MediaReportSink::channel(256);
    let lease = GstreamerMediaFactory::new(audio.devices(spec.real_audio), ReceiveSlots::default())
        .start(
            CallAttemptId::from_raw(1),
            Vec::new(),
            InitialMediaSettings::default(),
            sink,
        )
        .await
        .map_err(|e| format!("{e:?}"))?;
    gateway
        .send(&Command::VoiceJoin {
            channel_id: FIXTURE_CHANNEL.into(),
        })
        .await
        .map_err(|e| e.to_string())?;

    let mut peer_id = None;
    let mut revision = 0_u64;
    let mut subscribed = false;
    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            report = reports.recv() => {
                let Some(report) = report else { return Err("media report stream ended".into()); };
                let msg = match report.kind {
                    MediaReportKind::LocalOffer { sdp } => Command::VoiceOffer { channel_id: FIXTURE_CHANNEL.into(), sdp },
                    MediaReportKind::LocalAnswer { sdp } => Command::VoiceAnswer { channel_id: FIXTURE_CHANNEL.into(), sdp },
                    MediaReportKind::LocalIceCandidate { candidate, sdp_mid, sdp_mline_index } => {
                        Command::VoiceIceCandidate { channel_id: FIXTURE_CHANNEL.into(), candidate, sdp_mid, sdp_mline_index }
                    }
                    MediaReportKind::TransportState(MediaTransportState::Connected) if !subscribed => {
                        subscribed = true;
                        for (who, kind) in &targets {
                            let wire_kind = if *kind == VideoKind::Screen { TrackKind::Screen } else { TrackKind::Video };
                            gateway.send(&Command::TrackSubscribe {
                                channel_id: FIXTURE_CHANNEL.into(),
                                publisher_id: who.to_string(),
                                kind: wire_kind,
                            }).await.map_err(|e| e.to_string())?;
                        }
                        continue;
                    }
                    MediaReportKind::Fatal(e) => return Err(format!("media fatal: {e:?}")),
                    _ => continue,
                };
                gateway.send(&msg).await.map_err(|e| e.to_string())?;
            }
            event = gateway.next() => {
                let event = event.map_err(|e| e.to_string())?.ok_or("gateway ended")?;
                let Event::Known(event) = event else { continue; };
                let cmd = match event {
                    KnownEvent::VoiceJoined(e) if e.channel_id == FIXTURE_CHANNEL => {
                        peer_id = Some(e.peer_id);
                        lease.send(MediaCommand::StartNegotiation { video_codec: VideoCodec::from_wire(&e.video_codec) }).map_err(|e| format!("{e:?}"))?;
                        continue;
                    }
                    KnownEvent::VoiceAnswer(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => MediaCommand::RemoteAnswer { sdp: e.sdp },
                    KnownEvent::VoiceOffer(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => MediaCommand::RemoteOffer { sdp: e.sdp },
                    KnownEvent::VoiceICECandidate(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => MediaCommand::RemoteIceCandidate {
                        candidate: e.candidate, sdp_mid: e.sdp_mid, sdp_mline_index: e.sdp_mline_index,
                    },
                    KnownEvent::VoiceAudioRoutes(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => {
                        let routes = e.routes.into_iter().filter_map(|route| {
                            Some(AudioRoute {
                                track_id: route.track_id,
                                user: route.user_id.parse().ok()?,
                            })
                        }).collect();
                        MediaCommand::SetAudioRoutes { revision: e.revision, routes }
                    }
                    KnownEvent::VoiceVideoRoutes(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) && e.revision > revision => {
                        revision = e.revision;
                        let slots = e.routes.into_iter().filter_map(|route| {
                            let participant: Uuid = route.user_id.parse().ok()?;
                            let kind = VideoKind::from_wire(&route.kind)?;
                            let dest = targets.iter().position(|t| *t == (participant, kind)).map(|i| destinations[i].clone() as Arc<_>);
                            let (Some(extension_id @ 1..=14), Some(assignment_id @ 1..)) = (route.rtp_route_ext_id, route.rtp_route_id) else {
                                return None;
                            };
                            Some(VideoSlotBinding {
                                track_id: route.track_id, participant, kind,
                                fence: Some(VideoRouteFence { extension_id, assignment_id }),
                                destination: dest,
                            })
                        }).collect();
                        MediaCommand::SetVideoSlotPlan { revision, slots }
                    }
                    _ => continue,
                };
                lease.send(cmd).map_err(|e| format!("{e:?}"))?;
            }
        }
    }
    lease.stop();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(report) = reports.recv().await {
            if matches!(report.kind, MediaReportKind::Drained) {
                break;
            }
        }
    })
    .await;
    let _ = gateway
        .send(&Command::VoiceLeave {
            channel_id: FIXTURE_CHANNEL.into(),
        })
        .await;
    let _ = gateway.close().await;
    Ok(())
}
