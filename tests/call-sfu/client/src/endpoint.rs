//! Drives one gateway and media lease and supplies synthetic capture and a bounded PCM output.

use super::Result;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as app;
use papo_api::{
    RestClient,
    gateway::{Command, Event, Gateway, KnownEvent, TrackKind},
};
use papo_call::{CpuVideoDestination, GstreamerMediaFactory, MediaDevices, ReceiveSlots};
use papo_sdk::{
    AudioRoute, CallAttemptId, CameraFacing, InitialMediaSettings, MediaCommand,
    MediaEngineFactory, MediaLease, MediaReport, MediaReportKind, MediaReportSink,
    MediaTransportState, Uuid, VideoCodec, VideoDestinationId, VideoKind, VideoRouteFence,
    VideoSlotBinding, VideoTarget,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;

type Publishers = HashMap<(Uuid, VideoKind), Arc<CpuVideoDestination>>;

struct Devices {
    tone_hz: f64,
    pattern: &'static str,
    pcm: mpsc::Sender<Vec<i16>>,
}

impl MediaDevices for Devices {
    fn microphone(&self) -> Option<gst::Element> {
        gst::ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .property("freq", self.tone_hz)
            .property("volume", 0.3f64)
            .build()
            .ok()
    }

    fn camera(&self, _: CameraFacing) -> Option<gst::Element> {
        gst::ElementFactory::make("videotestsrc")
            .property("is-live", true)
            .property_from_str("pattern", self.pattern)
            .build()
            .ok()
    }

    fn speaker(&self) -> Option<gst::Element> {
        let sender = self.pcm.clone();
        Some(
            app::AppSink::builder()
                .sync(false)
                .async_(false)
                .max_buffers(1)
                .drop(true)
                .caps(
                    &gst::Caps::builder("audio/x-raw")
                        .field("format", "S16LE")
                        .field("layout", "interleaved")
                        .field("rate", 48_000i32)
                        .field("channels", 2i32)
                        .build(),
                )
                .callbacks(
                    app::AppSinkCallbacks::builder()
                        .new_sample(move |sink| {
                            let sample =
                                sink.pull_sample().map_err(|_| gst::FlowError::Flushing)?;
                            let data = sample
                                .buffer()
                                .ok_or(gst::FlowError::Error)?
                                .map_readable()
                                .map_err(|_| gst::FlowError::Error)?;
                            let samples = data
                                .as_slice()
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
                                .collect();
                            let _ = sender.try_send(samples);
                            Ok(gst::FlowSuccess::Ok)
                        })
                        .build(),
                )
                .build()
                .upcast(),
        )
    }
}

pub(super) struct Endpoint {
    gateway: Gateway,
    pub(super) lease: Box<dyn MediaLease>,
    reports: mpsc::Receiver<MediaReport>,
    pub(super) pcm: mpsc::Receiver<Vec<i16>>,
    channel: String,
    peer: Option<String>,
    pub(super) codec: Option<VideoCodec>,
    pub(super) connected: bool,
    pub(super) answer_count: u32,
    pub(super) route_revision: u64,
    pub(super) audio_revision: u64,
    pub(super) video: Publishers,
    pub(super) rejected_subscriptions: u32,
    pub(super) publisher_tracks: HashMap<(Uuid, VideoKind), (String, Option<u16>)>,
}

impl Endpoint {
    pub(super) async fn join(
        rest: &RestClient,
        channel: &str,
        tone_hz: f64,
        pattern: &'static str,
    ) -> Result<Self> {
        Self::rejoin(Gateway::connect(rest).await?, channel, tone_hz, pattern).await
    }

    pub(super) async fn rejoin(
        mut gateway: Gateway,
        channel: &str,
        tone_hz: f64,
        pattern: &'static str,
    ) -> Result<Self> {
        let (sink, reports) = MediaReportSink::channel(256);
        let (pcm_tx, pcm) = mpsc::channel(32);
        let factory = GstreamerMediaFactory::new(
            Arc::new(Devices {
                pattern,
                tone_hz,
                pcm: pcm_tx,
            }),
            ReceiveSlots::default(),
        );
        let lease = factory
            .start(
                CallAttemptId::from_raw(1),
                Vec::new(),
                InitialMediaSettings::default(),
                sink,
            )
            .await?;
        gateway
            .send(&Command::VoiceJoin {
                channel_id: channel.into(),
            })
            .await?;
        Ok(Self {
            gateway,
            channel: channel.into(),
            lease,
            reports,
            pcm,
            connected: false,
            peer: None,
            codec: None,
            answer_count: 0,
            route_revision: 0,
            audio_revision: 0,
            video: HashMap::new(),
            rejected_subscriptions: 0,
            publisher_tracks: HashMap::new(),
        })
    }

    pub(super) async fn step(&mut self) -> Result {
        tokio::select! {
            event = self.gateway.next() => {
                if let Some(Event::Known(event)) = event? {
                    self.receive(event).await?;
                }
            }
            report = self.reports.recv() => {
                let report = report.ok_or("media report stream closed")?;
                if let Some(command) = self.report(report.kind)? {
                    self.gateway.send(&command).await?;
                }
            }
        }
        Ok(())
    }

    fn report(&mut self, report: MediaReportKind) -> Result<Option<Command>> {
        let channel_id = self.channel.clone();
        let command = match report {
            MediaReportKind::LocalOffer { sdp } => {
                if self.answer_count == 0 {
                    assert_eq!(sdp.lines().filter(|l| l.starts_with("m=video ")).count(), 6);
                    assert_eq!(sdp.lines().filter(|l| l.starts_with("m=audio ")).count(), 9);
                    assert_eq!(
                        sdp.lines()
                            .find(|l| l.starts_with("m="))
                            .and_then(|line| line.split_whitespace().next()),
                        Some("m=audio")
                    );
                    assert!(sdp.contains("urn:ietf:params:rtp-hdrext:ssrc-audio-level"));
                }
                Command::VoiceOffer { channel_id, sdp }
            }
            MediaReportKind::LocalAnswer { sdp } => Command::VoiceAnswer { channel_id, sdp },
            MediaReportKind::LocalIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => Command::VoiceIceCandidate {
                channel_id,
                candidate,
                sdp_mid,
                sdp_mline_index,
            },
            MediaReportKind::TransportState(MediaTransportState::Connected) => {
                self.connected = true;
                return Ok(None);
            }
            MediaReportKind::Fatal(error) => return Err(format!("media fatal: {error:?}").into()),
            _ => return Ok(None),
        };
        Ok(Some(command))
    }

    async fn receive(&mut self, event: KnownEvent) -> Result {
        let command = match event {
            KnownEvent::VoiceJoined(event) => {
                let codec = VideoCodec::from_wire(&event.video_codec);
                self.codec = Some(codec);
                self.peer = Some(event.peer_id);
                MediaCommand::StartNegotiation { video_codec: codec }
            }
            KnownEvent::VoiceAnswer(event) if self.is_peer(&event.peer_id) => {
                self.answer_count += 1;
                MediaCommand::RemoteAnswer { sdp: event.sdp }
            }
            KnownEvent::VoiceOffer(event) if self.is_peer(&event.peer_id) => {
                MediaCommand::RemoteOffer { sdp: event.sdp }
            }
            KnownEvent::VoiceICECandidate(event) if self.is_peer(&event.peer_id) => {
                MediaCommand::RemoteIceCandidate {
                    candidate: event.candidate,
                    sdp_mid: event.sdp_mid,
                    sdp_mline_index: event.sdp_mline_index,
                }
            }
            KnownEvent::VoiceAudioRoutes(event) if self.is_peer(&event.peer_id) => {
                self.audio_revision = event.revision;
                let routes = event
                    .routes
                    .into_iter()
                    .map(|route| {
                        Ok(AudioRoute {
                            track_id: route.track_id,
                            user: route.user_id.parse()?,
                        })
                    })
                    .collect::<Result<_>>()?;
                MediaCommand::SetAudioRoutes {
                    revision: event.revision,
                    routes,
                }
            }
            KnownEvent::VoiceVideoRoutes(event)
                if self.is_peer(&event.peer_id) && event.revision > self.route_revision =>
            {
                self.route_revision = event.revision;
                self.publisher_tracks.clear();
                let slots = event
                    .routes
                    .into_iter()
                    .map(|route| {
                        let participant = route.user_id.parse()?;
                        let kind =
                            VideoKind::from_wire(&route.kind).ok_or("unexpected video kind")?;
                        self.publisher_tracks.insert(
                            (participant, kind),
                            (route.track_id.clone(), route.first_seq),
                        );
                        Ok(VideoSlotBinding {
                            track_id: route.track_id,
                            participant,
                            kind,
                            fence: match (route.rtp_route_ext_id, route.rtp_route_id) {
                                (Some(extension_id @ 1..=14), Some(assignment_id @ 1..)) => {
                                    Some(VideoRouteFence {
                                        extension_id,
                                        assignment_id,
                                    })
                                }
                                _ => None,
                            },
                            destination: self
                                .video
                                .get(&(participant, kind))
                                .cloned()
                                .map(|dest| dest as Arc<_>),
                        })
                    })
                    .collect::<Result<_>>()?;
                MediaCommand::SetVideoSlotPlan {
                    revision: event.revision,
                    slots,
                }
            }
            KnownEvent::Error(event) if event.code.as_deref() == Some("voice-not-found") => {
                self.rejected_subscriptions += 1;
                return Ok(());
            }
            KnownEvent::Error(event)
                if event
                    .code
                    .as_deref()
                    .is_some_and(|code| code.starts_with("voice-")) =>
            {
                return Err(format!("backend rejected voice operation ({:?})", event.code).into());
            }
            _ => return Ok(()),
        };
        self.lease.send(command)?;
        Ok(())
    }

    fn is_peer(&self, peer: &str) -> bool {
        self.peer.as_deref() == Some(peer)
    }

    pub(super) async fn camera_on(&mut self) -> Result {
        self.gateway
            .send(&Command::VoiceCamera {
                channel_id: self.channel.clone(),
                on: true,
            })
            .await?;
        self.lease.send(MediaCommand::SetCamera { on: true })?;
        Ok(())
    }

    pub(super) async fn mute(&mut self, muted: bool) -> Result {
        self.gateway
            .send(&Command::VoiceMute {
                channel_id: self.channel.clone(),
                muted,
            })
            .await?;
        self.lease.send(MediaCommand::SetMuted { muted })?;
        Ok(())
    }

    pub(super) async fn interest(&mut self, who: Uuid, kind: VideoKind, on: bool) -> Result {
        let wire_kind = if kind == VideoKind::Screen {
            TrackKind::Screen
        } else {
            TrackKind::Video
        };
        let channel_id = self.channel.clone();
        let publisher_id = who.to_string();
        let command = if on {
            let id = VideoDestinationId::from_raw(u64::try_from(self.video.len())? + 1);
            self.video.entry((who, kind)).or_insert_with(|| {
                Arc::new(CpuVideoDestination::new(
                    id,
                    VideoTarget {
                        participant: who,
                        kind,
                    },
                ))
            });
            Command::TrackSubscribe {
                channel_id,
                publisher_id,
                kind: wire_kind,
            }
        } else {
            Command::TrackUnsubscribe {
                channel_id,
                publisher_id,
                kind: wire_kind,
            }
        };
        self.gateway.send(&command).await?;
        Ok(())
    }

    pub(super) async fn leave(self) -> Result {
        self.leave_open().await?.close().await?;
        Ok(())
    }

    pub(super) async fn leave_open(mut self) -> Result<Gateway> {
        self.gateway
            .send(&Command::VoiceLeave {
                channel_id: self.channel.clone(),
            })
            .await?;
        self.lease.stop();
        while let Some(report) = self.reports.recv().await {
            if matches!(report.kind, MediaReportKind::Drained) {
                return Ok(self.gateway);
            }
        }
        Err("media stopped without drain".into())
    }
}
