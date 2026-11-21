//! Drives one or two fixed synthetic SFU publishers through papo-call with explicit drain.

use crate::{
    verify::ExpectedColor,
    writer::{FrameWork, TileWriter},
};
use gstreamer as gst;
use papo_api::{
    RestClient,
    gateway::{Command, Event, Gateway, KnownEvent, TrackKind},
    wire::LoginRequest,
};
use papo_call::{
    CpuVideoDestination, CpuVideoFrameLease, GstreamerMediaFactory, MediaDevices, ReceiveSlots,
    VideoRotation,
};
use papo_sdk::{
    CallAttemptId, CameraFacing, InitialMediaSettings, MediaCommand, MediaEngineFactory,
    MediaLease, MediaReport, MediaReportKind, MediaReportSink, MediaTransportState, Uuid,
    VideoCodec, VideoDestination as _, VideoDestinationId, VideoKind, VideoRouteFence,
    VideoSlotBinding, VideoTarget,
};
use std::{error::Error, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};

const CHANNEL: &str = "00000000-0000-4000-8000-000000000010";
const RED: &str = "00000000-0000-4000-8000-000000000101";
const GREEN: &str = "00000000-0000-4000-8000-000000000102";
const FIXTURE: &str = "http://127.0.0.1:18765";

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct NoDevices;

impl MediaDevices for NoDevices {
    fn microphone(&self) -> Option<gst::Element> {
        None
    }

    fn camera(&self, _: CameraFacing) -> Option<gst::Element> {
        None
    }

    fn speaker(&self) -> Option<gst::Element> {
        gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .ok()
    }
}

struct TileSlot {
    user_str: &'static str,
    participant: Uuid,
    expected: ExpectedColor,
    dest: Arc<CpuVideoDestination>,
    writer: Arc<TileWriter<CpuVideoFrameLease>>,
    active_route: Option<(String, u64)>,
}

pub(crate) async fn run(
    writers: &[Arc<TileWriter<CpuVideoFrameLease>>],
    mut cancel: oneshot::Receiver<()>,
) -> Result<()> {
    let specs: &[(&'static str, ExpectedColor)] = match writers.len() {
        1 => &[(RED, ExpectedColor::Red)],
        2 => &[(RED, ExpectedColor::Red), (GREEN, ExpectedColor::Green)],
        count => return Err(format!("unsupported tile count: {count}").into()),
    };

    let mut tiles = Vec::with_capacity(specs.len());
    for (idx, (user_str, expected)) in specs.iter().enumerate() {
        let participant: Uuid = user_str.parse()?;
        let dest = Arc::new(CpuVideoDestination::new(
            VideoDestinationId::from_raw(1 + idx as u64),
            VideoTarget {
                participant,
                kind: VideoKind::Camera,
            },
        ));
        tiles.push(TileSlot {
            user_str,
            participant,
            expected: *expected,
            dest,
            writer: Arc::clone(&writers[idx]),
            active_route: None,
        });
    }

    let mut gateway = tokio::select! {
        _ = &mut cancel => return Ok(()),
        conn = connect_gateway() => conn?,
    };

    let (sink, mut reports) = MediaReportSink::channel(128);
    let lease = GstreamerMediaFactory::new(Arc::new(NoDevices), ReceiveSlots::default())
        .start(
            CallAttemptId::from_raw(1),
            Vec::new(),
            InitialMediaSettings::default(),
            sink,
        )
        .await?;

    let loop_result = drive_session(
        &mut gateway,
        lease.as_ref(),
        &mut reports,
        &mut tiles,
        &mut cancel,
    )
    .await;

    for tile in &tiles {
        tile.dest.revoke();
    }
    lease.stop();
    while let Some(report) = reports.recv().await {
        if matches!(report.kind, MediaReportKind::Drained) {
            break;
        }
    }
    drop(lease);
    drop(gateway);
    loop_result
}

async fn connect_gateway() -> Result<Gateway> {
    let rest = RestClient::loopback(FIXTURE)?;
    rest.login(&LoginRequest {
        username: "fixture".into(),
        password: String::new(),
    })
    .await?;
    Ok(Gateway::connect(&rest).await?)
}

async fn drive_session(
    gateway: &mut Gateway,
    lease: &dyn MediaLease,
    reports: &mut mpsc::Receiver<MediaReport>,
    tiles: &mut [TileSlot],
    cancel: &mut oneshot::Receiver<()>,
) -> Result<()> {
    gateway
        .send(&Command::VoiceJoin {
            channel_id: CHANNEL.into(),
        })
        .await?;

    let mut peer_id = None;
    let mut revision = 0;
    let mut next_dest_id = 1 + tiles.len() as u64;
    let mut subscribed = false;
    let mut first_frame_offered = false;
    let mut cadence = tokio::time::interval(Duration::from_millis(33));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let first_frame_deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(first_frame_deadline);

    loop {
        tokio::select! {
            _ = &mut *cancel => return Ok(()),
            report = reports.recv() => {
                let report = report.ok_or("media report stream ended")?;
                match report.kind {
                    MediaReportKind::LocalOffer { sdp } => {
                        gateway.send(&Command::VoiceOffer { channel_id: CHANNEL.into(), sdp }).await?;
                    }
                    MediaReportKind::LocalAnswer { sdp } => {
                        gateway.send(&Command::VoiceAnswer { channel_id: CHANNEL.into(), sdp }).await?;
                    }
                    MediaReportKind::LocalIceCandidate { candidate, sdp_mid, sdp_mline_index } => {
                        gateway.send(&Command::VoiceIceCandidate {
                            channel_id: CHANNEL.into(),
                            candidate,
                            sdp_mid,
                            sdp_mline_index,
                        }).await?;
                    }
                    MediaReportKind::TransportState(MediaTransportState::Connected) if !subscribed => {
                        subscribed = true;
                        for tile in tiles.iter() {
                            gateway.send(&Command::TrackSubscribe {
                                channel_id: CHANNEL.into(),
                                publisher_id: tile.user_str.into(),
                                kind: TrackKind::Video,
                            }).await?;
                        }
                    }
                    MediaReportKind::Fatal(_) => return Err("media engine failed".into()),
                    _ => {}
                }
            }
            event = gateway.next() => {
                let event = event?.ok_or("fixture connection ended")?;
                let Event::Known(event) = event else { continue; };
                let command = match event {
                    KnownEvent::VoiceJoined(e) if e.channel_id == CHANNEL => {
                        peer_id = Some(e.peer_id);
                        lease.send(MediaCommand::StartNegotiation {
                            video_codec: VideoCodec::from_wire(&e.video_codec),
                        })?;
                        continue;
                    }
                    KnownEvent::VoiceAnswer(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => {
                        MediaCommand::RemoteAnswer { sdp: e.sdp }
                    }
                    KnownEvent::VoiceOffer(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => {
                        MediaCommand::RemoteOffer { sdp: e.sdp }
                    }
                    KnownEvent::VoiceICECandidate(e) if peer_id.as_deref() == Some(e.peer_id.as_str()) => {
                        MediaCommand::RemoteIceCandidate {
                            candidate: e.candidate,
                            sdp_mid: e.sdp_mid,
                            sdp_mline_index: e.sdp_mline_index,
                        }
                    }
                    KnownEvent::VoiceVideoRoutes(e)
                        if peer_id.as_deref() == Some(e.peer_id.as_str()) && e.revision > revision =>
                    {
                        revision = e.revision;
                        let mut assigned: Vec<Option<(String, u64, u8)>> = vec![None; tiles.len()];
                        for route in e.routes {
                            if VideoKind::from_wire(&route.kind) != Some(VideoKind::Camera) {
                                continue;
                            }
                            let (Some(extension_id @ 1..=14), Some(assignment_id @ 1..)) =
                                (route.rtp_route_ext_id, route.rtp_route_id)
                            else {
                                continue;
                            };
                            if let Some(idx) = tiles.iter().position(|t| t.user_str == route.user_id)
                            {
                                assigned[idx] =
                                    Some((route.track_id, assignment_id, extension_id));
                            }
                        }
                        let mut slots = Vec::new();
                        for (tile, next_route) in tiles.iter_mut().zip(assigned) {
                            let next_key = next_route
                                .as_ref()
                                .map(|(track_id, assignment_id, _)| {
                                    (track_id.clone(), *assignment_id)
                                });
                            if next_key != tile.active_route {
                                // Revoke old leases before replacing the destination so in-flight frames drop cleanly.
                                tile.dest.revoke();
                                tile.dest = Arc::new(CpuVideoDestination::new(
                                    VideoDestinationId::from_raw(next_dest_id),
                                    VideoTarget {
                                        participant: tile.participant,
                                        kind: VideoKind::Camera,
                                    },
                                ));
                                next_dest_id += 1;
                                tile.active_route = next_key;
                                if tile.active_route.is_none() {
                                    tile.writer.mark_unrouted();
                                }
                            }
                            if let Some((track_id, assignment_id, extension_id)) = next_route {
                                slots.push(VideoSlotBinding {
                                    track_id,
                                    participant: tile.participant,
                                    kind: VideoKind::Camera,
                                    fence: Some(VideoRouteFence {
                                        extension_id,
                                        assignment_id,
                                    }),
                                    destination: Some(tile.dest.clone()),
                                });
                            }
                        }
                        MediaCommand::SetVideoSlotPlan { revision, slots }
                    }
                    _ => continue,
                };
                lease.send(command)?;
            }
            _ = &mut first_frame_deadline, if !first_frame_offered => {
                return Err("first frame timed out".into());
            }
            _ = cadence.tick() => {
                for tile in tiles.iter_mut() {
                    let Some((track_id, assignment_id)) = tile.active_route.clone() else {
                        continue;
                    };
                    if let Some(lease) = tile.dest.take()
                        && lease.is_current()
                        && let Ok(bgra) = lease.pack_bgra(VideoRotation::Upright)
                        && lease.is_current()
                    {
                        tile.writer.offer(FrameWork {
                            lease,
                            bgra,
                            track_id,
                            assignment_id,
                            expected: tile.expected,
                        });
                        first_frame_offered = true;
                    }
                }
            }
        }
    }
}
