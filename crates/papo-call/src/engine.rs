//! Implements the SDK lease with a bounded FIFO and separate stop signal. GStreamer state and
//! teardown live on a blocking worker, whose supervisor reports drain after the worker exits.

use crate::{
    camera::Camera,
    diagnostics,
    ice::IceConfig,
    mic::Mic,
    negotiation::{Candidate, Codec, Negotiation, connect_ice, opus_caps, wait_for_rtp},
    pipeline::{Branch, MediaResult, PadProbe, element},
    platform::MediaDevices,
    receive_audio::ReceiveAudio,
    receive_video::ReceiveVideo,
    slots::ReceiveSlots,
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_webrtc as rtc;
use papo_sdk::{
    CallAttemptId, CaptureState, IceServerConfig, InitialMediaSettings, MediaCommand,
    MediaFatalError, MediaReport, MediaReportKind, MediaReportSink, MediaStartError,
    MediaTransportState, MediaWarning,
};
use std::sync::Arc;
use tokio::{runtime::Handle, sync::mpsc};
use tokio_util::sync::CancellationToken;

pub(crate) const QUEUE_CAPACITY: usize = 128;

enum Event {
    Report(MediaReportKind),
    PadAdded(gst::Pad, PadProbe),
    PadRemoved(gst::Pad),
    Failure(Option<gst::Object>),
}

pub(crate) struct Engine {
    graph: gst::Pipeline,
    negotiation: Negotiation,
    devices: Arc<dyn MediaDevices>,
    mic: Mic,
    camera: Camera,
    audio: ReceiveAudio,
    video: ReceiveVideo,
    drains: std::collections::HashMap<gst::Pad, Branch>,
    events: mpsc::Receiver<Event>,
    overflow: CancellationToken,
    settings: InitialMediaSettings,
    camera_pending: bool,
    slots: ReceiveSlots,
    codec: Option<Codec>,
}

impl Engine {
    pub fn new(
        devices: Arc<dyn MediaDevices>,
        slots: ReceiveSlots,
        ice: Vec<IceServerConfig>,
        settings: InitialMediaSettings,
        stop: &CancellationToken,
    ) -> Result<Self, MediaStartError> {
        ensure_start_active(stop)?;
        diagnostics::initialize()?;
        ensure_start_active(stop)?;
        if !slots.is_supported() {
            return Err(MediaStartError::Internal);
        }
        let graph = gst::Pipeline::new();
        let peer = element("webrtcbin").map_err(|_| MediaStartError::DeviceUnavailable)?;
        peer.set_property_from_str("bundle-policy", "max-bundle");
        peer.set_property("latency", 120u32);
        let ice = IceConfig::from_servers(&ice);
        if let Some(stun) = &ice.stun {
            peer.set_property("stun-server", stun);
        }
        for turn in &ice.turn {
            if !peer.emit_by_name::<bool>("add-turn-server", &[turn]) {
                log::warn!("webrtcbin rejected a TURN server configuration");
            }
        }
        graph.add(&peer).map_err(|_| MediaStartError::Internal)?;
        let (tx, events) = mpsc::channel(QUEUE_CAPACITY);
        let overflow = CancellationToken::new();
        connect(&graph, &peer, tx, overflow.clone())?;
        ensure_start_active(stop)?;
        let mic = Mic::new(&graph, &peer).map_err(|_| MediaStartError::DeviceUnavailable)?;
        let sink = devices
            .speaker()
            .ok_or(MediaStartError::DeviceUnavailable)?;
        let audio =
            ReceiveAudio::new(&graph, sink).map_err(|_| MediaStartError::DeviceUnavailable)?;
        let mut engine = Self {
            graph,
            negotiation: Negotiation::new(peer),
            devices,
            mic,
            audio,
            events,
            overflow,
            settings,
            slots,
            camera: Camera::default(),
            video: ReceiveVideo::default(),
            drains: Default::default(),
            camera_pending: false,
            codec: None,
        };
        ensure_start_active(stop)?;
        if engine.graph.set_state(gst::State::Playing).is_err() {
            engine.drain(None);
            return Err(MediaStartError::DeviceUnavailable);
        }
        if stop.is_cancelled() {
            engine.drain(None);
            return Err(MediaStartError::Cancelled);
        }
        Ok(engine)
    }

    pub async fn run(
        &mut self,
        mut commands: mpsc::Receiver<MediaCommand>,
        stop: &CancellationToken,
        attempt: CallAttemptId,
        reports: &MediaReportSink,
    ) -> MediaResult<()> {
        loop {
            let overflow = self.overflow.clone();
            tokio::select! {
                biased;
                () = stop.cancelled() => return Ok(()),
                () = overflow.cancelled() => return Err(MediaFatalError::Internal),
                event = self.events.recv() => {
                    let Some(event) = event else { return Err(MediaFatalError::Internal) };
                    send_reports(reports, attempt, self.event(event).await?)?;
                }
                command = commands.recv() => {
                    let Some(command) = command else { return Ok(()) };
                    // Allow an idempotent stop even while a GStreamer promise or idle
                    // probe is waiting. Dropping the operation does not drop graph ownership.
                    let update = tokio::select! {
                        biased;
                        () = stop.cancelled() => return Ok(()),
                        result = self.command(command) => result?,
                    };
                    send_reports(reports, attempt, update)?;
                }
            }
        }
    }

    async fn event(&mut self, event: Event) -> MediaResult<Vec<MediaReportKind>> {
        match event {
            Event::Report(report) => return Ok(vec![report]),
            Event::PadAdded(pad, _block) => {
                if let Some((video, track)) = self.negotiation.track(&pad) {
                    if video {
                        let codec = self.codec.ok_or(MediaFatalError::UnsupportedCodec)?;
                        self.video.attach(&self.graph, &pad, track, codec)?;
                    } else {
                        self.audio.attach(&self.graph, &pad, track)?;
                    }
                } else {
                    let sink = element("fakesink")?;
                    sink.set_property("async", false);
                    sink.set_property("sync", false);
                    let drain = Branch::attach(&self.graph, vec![sink], Some(&pad), None)?;
                    self.drains.insert(pad, drain);
                }
            }
            Event::PadRemoved(pad) => {
                self.video.detach(&pad);
                self.audio.detach(&pad);
                self.drains.remove(&pad);
            }
            Event::Failure(Some(source)) if self.mic.owns_capture(&source) => {
                self.mic.release();
                return Ok(vec![
                    MediaReportKind::MicCapture(CaptureState::Unavailable),
                    MediaReportKind::Warning(MediaWarning::MicrophoneUnavailable),
                ]);
            }
            Event::Failure(Some(source)) if self.camera.owns_capture(&source) => {
                self.camera.release();
                self.settings.camera_on = false;
                self.camera_pending = true;
                let mut reports = Vec::new();
                self.update_camera(&mut reports).await?;
                reports.retain(|report| !matches!(report, MediaReportKind::CameraCapture(_)));
                reports.insert(0, MediaReportKind::CameraCapture(CaptureState::Unavailable));
                reports.insert(1, MediaReportKind::Warning(MediaWarning::CameraUnavailable));
                return Ok(reports);
            }
            Event::Failure(Some(source))
                if source != *self.graph.upcast_ref::<gst::Object>()
                    && !source.has_as_ancestor(&self.graph) => {}
            Event::Failure(_) => return Err(MediaFatalError::DeviceError),
        }
        Ok(Vec::new())
    }

    async fn command(&mut self, command: MediaCommand) -> MediaResult<Vec<MediaReportKind>> {
        let mut reports = Vec::new();
        match command {
            MediaCommand::StartNegotiation { video_codec } => {
                if self.codec.is_some() {
                    return Err(MediaFatalError::NegotiationFailed);
                }
                let codec = Codec::try_from(video_codec)?;
                self.codec = Some(codec);
                for _ in 0..self.slots.video {
                    self.negotiation.receive_line(&codec.caps())?;
                }
                for _ in 0..self.slots.audio {
                    self.negotiation.receive_line(&opus_caps())?;
                }
                self.mic_capture(&mut reports);
                wait_for_rtp(&self.mic.rtp).await?;
                reports.push(self.negotiation.offer().await?);
            }
            MediaCommand::RemoteAnswer { ref sdp } | MediaCommand::RemoteOffer { ref sdp } => {
                let is_offer = matches!(command, MediaCommand::RemoteOffer { .. });
                let codec = self.codec.ok_or(MediaFatalError::NegotiationFailed)?;
                reports.extend(self.negotiation.apply_remote(sdp, is_offer, codec).await?);
                self.update_camera(&mut reports).await?;
            }
            MediaCommand::RemoteIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                self.negotiation.candidate(Candidate {
                    text: candidate,
                    mid: sdp_mid,
                    index: sdp_mline_index,
                })?;
            }
            MediaCommand::SetMuted { muted } => {
                self.settings.muted = muted;
                self.mic_capture(&mut reports);
            }
            MediaCommand::SetCamera { on } => {
                self.camera_pending |= self.settings.camera_on != on;
                self.settings.camera_on = on;
                if !on {
                    self.camera.release();
                }
                self.update_camera(&mut reports).await?;
            }
            MediaCommand::SetCameraFacing { facing } => {
                self.camera_pending |=
                    self.settings.camera_on && self.settings.camera_facing != facing;
                self.settings.camera_facing = facing;
                self.update_camera(&mut reports).await?;
            }
            MediaCommand::SetPlayback { user, playback } => self.audio.playback(user, playback),
            MediaCommand::SetAudioRoutes { revision, routes } => self.audio.route(revision, routes),
            MediaCommand::SetVideoSlotPlan { revision, slots } => {
                self.video
                    .plan(&self.graph, self.codec, revision, slots)
                    .await?;
            }
            _ => return Err(MediaFatalError::Internal),
        }
        Ok(reports)
    }

    fn mic_capture(&mut self, reports: &mut Vec<MediaReportKind>) {
        let state = self
            .mic
            .set_muted(&self.graph, self.devices.as_ref(), self.settings.muted);
        reports.push(MediaReportKind::MicCapture(state));
        if state == CaptureState::Unavailable {
            reports.push(MediaReportKind::Warning(
                MediaWarning::MicrophoneUnavailable,
            ));
        }
    }

    async fn update_camera(&mut self, reports: &mut Vec<MediaReportKind>) -> MediaResult<()> {
        if !self.negotiation.established() || self.negotiation.waiting() {
            return Ok(());
        }
        if !self.camera_pending && self.camera.publishing() == self.settings.camera_on {
            return Ok(());
        }
        self.camera_pending = false;
        let codec = self.codec.ok_or(MediaFatalError::Internal)?;
        let (state, update) = self.camera.set(
            &self.graph,
            &self.negotiation.webrtc,
            codec,
            self.settings.camera_on.then_some(self.devices.as_ref()),
            self.settings.camera_facing,
        );
        if update
            && state == CaptureState::Active
            && let Some(pad) = self.camera.rtp()
            && wait_for_rtp(pad).await.is_err()
        {
            // The capture device or encoder can fail after accepting Playing.
            // The unoffered line can stay inactive while audio continues.
            self.camera.set(
                &self.graph,
                &self.negotiation.webrtc,
                codec,
                None,
                self.settings.camera_facing,
            );
            reports.push(MediaReportKind::CameraCapture(CaptureState::Unavailable));
            reports.push(MediaReportKind::Warning(MediaWarning::CameraUnavailable));
            return Ok(());
        }
        reports.push(MediaReportKind::CameraCapture(state));
        if state == CaptureState::Unavailable {
            reports.push(MediaReportKind::Warning(MediaWarning::CameraUnavailable));
        }
        if update {
            reports.push(self.negotiation.offer().await?);
        }
        Ok(())
    }

    pub fn drain(&mut self, runtime: Option<&Handle>) -> bool {
        self.video.revoke();
        self.camera.release();
        self.mic.release();
        self.events.close();
        while self.events.try_recv().is_ok() {}
        if let Some(runtime) = runtime {
            runtime.block_on(self.negotiation.close());
        }
        let state = self.graph.set_state(gst::State::Null);
        self.video.drain();
        self.audio.drain();
        self.drains.clear();
        if let Some(bus) = self.graph.bus() {
            bus.unset_sync_handler();
        }
        state.is_ok()
    }
}

fn ensure_start_active(stop: &CancellationToken) -> Result<(), MediaStartError> {
    if stop.is_cancelled() {
        return Err(MediaStartError::Cancelled);
    }
    Ok(())
}

fn send_reports(
    reports: &MediaReportSink,
    attempt: CallAttemptId,
    batch: Vec<MediaReportKind>,
) -> MediaResult<()> {
    for kind in batch {
        reports
            .try_send(MediaReport { attempt, kind })
            .map_err(|_| MediaFatalError::Internal)?;
    }
    Ok(())
}

fn connect(
    graph: &gst::Pipeline,
    peer: &gst::Element,
    tx: mpsc::Sender<Event>,
    overflow: CancellationToken,
) -> Result<(), MediaStartError> {
    let push = move |event| {
        if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(event) {
            overflow.cancel();
        }
    };
    let ice = push.clone();
    connect_ice(peer, move |report| ice(Event::Report(report)));
    let state = push.clone();
    peer.connect_notify(Some("connection-state"), move |peer, _| {
        use rtc::WebRTCPeerConnectionState as S;
        let state_value = match peer.property::<S>("connection-state") {
            S::Connecting => MediaTransportState::Connecting,
            S::Connected => MediaTransportState::Connected,
            S::Disconnected => MediaTransportState::Disconnected,
            S::Failed => MediaTransportState::Failed,
            S::Closed => MediaTransportState::Closed,
            _ => MediaTransportState::New,
        };
        state(Event::Report(MediaReportKind::TransportState(state_value)));
    });
    let added = push.clone();
    peer.connect_pad_added(move |_, pad| {
        if pad.direction() == gst::PadDirection::Src {
            added(Event::PadAdded(pad.clone(), PadProbe::block(pad)));
        }
    });
    let removed = push.clone();
    peer.connect_pad_removed(move |_, pad| {
        if pad.direction() == gst::PadDirection::Src {
            removed(Event::PadRemoved(pad.clone()));
        }
    });
    let bus = graph.bus().ok_or(MediaStartError::Internal)?;
    bus.set_sync_handler(move |_, message| {
        if let gst::MessageView::Error(error) = message.view() {
            diagnostics::graph_error(error);
            push(Event::Failure(message.src().cloned()));
        }
        gst::BusSyncReply::Drop
    });
    Ok(())
}
