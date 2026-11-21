//! Receives SDP-identified SFU slots. SDK routes alone select a participant's output. Route changes
//! discard the old decoder; each branch only accepts packets for its own assignment.

use crate::{
    camera::android_codecs,
    cpu_video::CpuVideoDestination,
    diagnostics,
    negotiation::Codec,
    pipeline::{
        Branch, MediaResult, PadProbe, RequestedPad, element, filter, pad, queue, ready_codec,
    },
    video_route::gate_decoder_input,
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as app;
use papo_sdk::{MediaFatalError, VideoDestination, VideoRouteFence, VideoSlotBinding, VideoTarget};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const RTP_INTAKE_QUEUE_NS: u64 = 250_000_000;
const RTP_INTAKE_MAX_BYTES: u32 = 4 * 1024 * 1024;

fn intake_queue() -> MediaResult<gst::Element> {
    // A single 720p/1080p keyframe bursts 30–60+ RTP packets at once; capping by small buffer
    // count tears frames before depayload and drops the whole GOP. Bound by time and bytes.
    let intake = queue(0, RTP_INTAKE_QUEUE_NS)?;
    intake.set_property("max-size-bytes", RTP_INTAKE_MAX_BYTES);
    Ok(intake)
}

#[derive(Default)]
pub(crate) struct ReceiveVideo {
    ports: HashMap<gst::Pad, Port>,
    plan: HashMap<String, VideoSlotBinding>,
    revision: u64,
}

struct Port {
    track: String,
    binding: Option<VideoSlotBinding>,
    branch: Option<Branch>,
    output: RequestedPad,
    _inlet: Branch,
    route_extension: Option<u8>,
}

impl ReceiveVideo {
    pub fn attach(
        &mut self,
        graph: &gst::Pipeline,
        pad: &gst::Pad,
        track: String,
        codec: Codec,
        route_extension: Option<u8>,
    ) -> MediaResult<()> {
        let binding = self.plan.get(&track);
        let tee = element("tee")?;
        tee.set_property("allow-not-linked", true);
        let inlet = Branch::attach(graph, vec![tee.clone()], Some(pad), None)?;
        let output = RequestedPad::new(&tee, "src_%u")?;
        let branch = branch(graph, &output.pad, codec, binding, route_extension)?;
        self.ports.insert(
            pad.clone(),
            Port {
                track,
                binding: binding.cloned(),
                branch,
                output,
                _inlet: inlet,
                route_extension,
            },
        );
        Ok(())
    }

    pub fn detach(&mut self, pad: &gst::Pad) {
        if let Some(port) = self.ports.remove(pad) {
            clear(port.binding.as_ref());
        }
    }

    pub async fn plan(
        &mut self,
        graph: &gst::Pipeline,
        codec: Option<Codec>,
        revision: u64,
        bindings: Vec<VideoSlotBinding>,
    ) -> MediaResult<()> {
        if revision <= self.revision {
            return Ok(());
        }
        self.revision = revision;
        self.plan = unambiguous_plan(bindings);
        let Some(codec) = codec else { return Ok(()) };
        // Some destinations move to a different slot in this same snapshot. Invalidate
        // every old generation before installing any new branch for those destinations.
        for port in self.ports.values() {
            if !equivalent(port.binding.as_ref(), self.plan.get(&port.track)) {
                clear(port.binding.as_ref());
            }
        }
        for (source, port) in &mut self.ports {
            let next = self.plan.get(&port.track);
            if equivalent(port.binding.as_ref(), next) {
                continue;
            }
            let _idle = PadProbe::idle(source).await?;
            port.branch = None;
            port.branch = branch(graph, &port.output.pad, codec, next, port.route_extension)?;
            port.binding = next.cloned();
        }
        Ok(())
    }

    pub fn revoke(&self) {
        let running = self.ports.values().filter_map(|port| port.binding.as_ref());
        for binding in self.plan.values().chain(running) {
            if let Some(destination) = &binding.destination {
                destination.revoke();
            }
        }
    }

    pub fn drain(&mut self) {
        for port in self.ports.values() {
            clear(port.binding.as_ref());
        }
        self.ports.clear();
        self.plan.clear();
    }
}

fn route_identity(
    route: Option<&VideoSlotBinding>,
) -> Option<(papo_sdk::Uuid, papo_sdk::VideoKind, Option<VideoRouteFence>)> {
    route.map(|route| (route.participant, route.kind, route.fence))
}

fn unambiguous_plan(bindings: Vec<VideoSlotBinding>) -> HashMap<String, VideoSlotBinding> {
    use std::collections::hash_map::Entry;

    let mut plan: HashMap<String, VideoSlotBinding> = HashMap::new();
    let mut seen = HashSet::new();
    let mut ambiguous = HashSet::new();
    let target = |binding: &VideoSlotBinding| VideoTarget {
        participant: binding.participant,
        kind: binding.kind,
    };
    for binding in bindings {
        let who = target(&binding);
        if !seen.insert(who) {
            ambiguous.insert(who);
        }
        match plan.entry(binding.track_id.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(binding);
            }
            Entry::Occupied(slot) => {
                ambiguous.insert(who);
                ambiguous.insert(target(slot.get()));
            }
        }
    }
    // An ambiguous route must not drive two decoders into one mailbox. A later
    // unambiguous snapshot reattaches from scratch, not to a stale generation.
    for binding in plan.values_mut() {
        if ambiguous.contains(&target(binding)) {
            binding.destination = None;
        }
    }
    plan
}

fn equivalent(previous: Option<&VideoSlotBinding>, next: Option<&VideoSlotBinding>) -> bool {
    fn destination(binding: Option<&VideoSlotBinding>) -> Option<&Arc<dyn VideoDestination>> {
        binding.and_then(|binding| binding.destination.as_ref())
    }
    let same_destination = match (destination(previous), destination(next)) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    route_identity(previous) == route_identity(next) && same_destination
}

fn cpu_destination(destination: &dyn VideoDestination) -> Option<&CpuVideoDestination> {
    destination.as_any().downcast_ref()
}

fn clear(binding: Option<&VideoSlotBinding>) {
    let Some(route) = binding else { return };
    if let Some(destination) = route.destination.as_deref().and_then(cpu_destination)
        && destination.matches(route)
    {
        destination.reset();
    }
}

fn branch(
    graph: &gst::Pipeline,
    source: &gst::Pad,
    codec: Codec,
    binding: Option<&VideoSlotBinding>,
    route_extension: Option<u8>,
) -> MediaResult<Option<Branch>> {
    let destination = binding.and_then(|route| {
        let fence = route.fence.filter(|fence| {
            fence.assignment_id != 0 && Some(fence.extension_id) == route_extension
        })?;
        let destination = route.destination.as_ref()?;
        let cpu = cpu_destination(destination.as_ref())?;
        if !cpu.matches(route) {
            diagnostics::video_destination_mismatch();
            return None;
        }
        Some((
            Arc::clone(destination),
            cpu.reset(),
            fence,
            VideoTarget {
                participant: route.participant,
                kind: route.kind,
            },
        ))
    });
    let Some((destination, generation, fence, target)) = destination else {
        let drain = element("fakesink")?;
        drain.set_property("async", false);
        drain.set_property("sync", false);
        return Branch::attach(graph, vec![drain], Some(source), None).map(Some);
    };

    let (depay, decoder) = match codec {
        Codec::Vp8 => (element("rtpvp8depay")?, element("vp8dec")?),
        Codec::H264 => (element("rtph264depay")?, h264_decoder()?),
    };
    if depay.has_property("request-keyframe") {
        depay.set_property("request-keyframe", true);
        depay.set_property("wait-for-keyframe", true);
    }
    let sink = app::AppSink::builder()
        .sync(false)
        .async_(false)
        .max_buffers(1)
        .drop(true)
        .callbacks(
            app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Flushing)?;
                    if let Some(cpu) = cpu_destination(destination.as_ref()) {
                        let accepted = cpu.put(generation, &sample);
                        #[cfg(feature = "benchmark-telemetry")]
                        if accepted {
                            crate::telemetry::record_decoded_frame();
                        }
                        #[cfg(not(feature = "benchmark-telemetry"))]
                        let _ = accepted;
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        )
        .build();
    let intake = intake_queue()?;
    gate_decoder_input(&pad(&intake, "sink")?, target, fence);
    let mut elements = vec![intake, depay];
    if codec == Codec::H264 {
        elements.push(element("h264parse")?);
    }
    elements.extend([
        decoder.clone(),
        element("videoconvert")?,
        filter(
            &gst::Caps::builder("video/x-raw")
                .field("format", "NV12")
                .build(),
        )?,
        sink.upcast(),
    ]);
    let result = Branch::attach(graph, elements, Some(source), None)?;
    diagnostics::codec_selected(&decoder, "decoder");
    #[cfg(feature = "benchmark-telemetry")]
    crate::telemetry::record_decoder(&decoder);
    Ok(Some(result))
}

fn h264_decoder() -> MediaResult<gst::Element> {
    let main = gst::Caps::builder("video/x-h264")
        .field("profile", "main")
        .build();
    let names = [
        "vah264dec",
        "vaapih264dec",
        "vtdec_hw",
        "d3d11h264dec",
        "mfh264dec",
    ];
    for name in names
        .into_iter()
        .map(str::to_owned)
        .chain(android_codecs("amcviddec-"))
    {
        if let Some(decoder) = ready_codec(&name, "sink", &main) {
            return Ok(decoder);
        }
    }
    element("openh264dec").map_err(|_| MediaFatalError::UnsupportedCodec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use papo_sdk::{Uuid, VideoDestinationId, VideoKind};

    fn binding() -> VideoSlotBinding {
        let target = VideoTarget {
            participant: Uuid::nil(),
            kind: VideoKind::Camera,
        };
        VideoSlotBinding {
            track_id: "video-0".into(),
            participant: target.participant,
            kind: target.kind,
            fence: Some(VideoRouteFence {
                extension_id: 14,
                assignment_id: 1,
            }),
            destination: Some(Arc::new(CpuVideoDestination::new(
                VideoDestinationId::from_raw(1),
                target,
            ))),
        }
    }

    #[test]
    fn ambiguous_slots_disable_the_destination_until_a_unique_route_returns() {
        let first = binding();
        let mut second = first.clone();
        second.track_id = "video-1".into();
        let ambiguous = unambiguous_plan(vec![first.clone(), second]);
        assert_eq!(ambiguous.len(), 2);
        assert!(ambiguous.values().all(|slot| slot.destination.is_none()));
        let unique = unambiguous_plan(vec![first]);
        assert!(unique["video-0"].destination.is_some());
        assert!(!equivalent(ambiguous.get("video-0"), unique.get("video-0")));
    }

    #[test]
    fn replacing_a_destination_with_the_same_id_still_rebuilds_the_decoder() {
        let original = binding();
        assert!(equivalent(Some(&original), Some(&original.clone())));
        assert!(!equivalent(Some(&original), Some(&binding())));
    }

    #[test]
    fn intake_queue_preserves_multi_packet_keyframe_bursts_without_tearing() {
        use gstreamer_rtp::prelude::RTPBufferExt as _;

        gst::init().unwrap();
        let pipeline = gst::Pipeline::new();
        let src = app::AppSrc::builder()
            .caps(
                &gst::Caps::builder("application/x-rtp")
                    .field("media", "video")
                    .field("clock-rate", 90_000_i32)
                    .field("encoding-name", "VP8")
                    .field("payload", 96_i32)
                    .build(),
            )
            .format(gst::Format::Time)
            .build();
        let intake = intake_queue().unwrap();
        let depay = element("rtpvp8depay").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = app::AppSink::builder()
            .sync(false)
            .async_(false)
            .callbacks(
                app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Flushing)?;
                        let _ = tx.send(sample.buffer().map_or(0, |b| b.size()));
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            )
            .build();
        pipeline
            .add_many([src.upcast_ref(), &intake, &depay, sink.upcast_ref()])
            .unwrap();
        gst::Element::link_many([src.upcast_ref(), &intake, &depay, sink.upcast_ref()]).unwrap();
        let block = PadProbe::block(&intake.static_pad("src").unwrap());
        pipeline.set_state(gst::State::Playing).unwrap();

        // Push a 40-packet VP8 keyframe burst (packets 0..39, same PTS=0, marker on packet 39).
        for idx in 0_u16..40 {
            let payload_len = if idx == 0 { 16 } else { 100 };
            let mut buffer = gst::Buffer::new_rtp_with_sizes(payload_len, 0, 0).unwrap();
            {
                let mut rtp =
                    gstreamer_rtp::RTPBuffer::from_buffer_writable(buffer.get_mut().unwrap())
                        .unwrap();
                rtp.set_payload_type(96);
                rtp.set_seq(100 + idx);
                rtp.set_timestamp(90_000);
                rtp.set_marker(idx == 39);
                let payload = rtp.payload_mut().unwrap();
                // RFC 7741 VP8 payload descriptor: S=1 (start of partition) on packet 0 only,
                // and VP8 uncompressed keyframe header (bit 0 == 0) in first frame byte.
                payload[0] = if idx == 0 { 0x10 } else { 0x00 };
                if idx == 0 {
                    payload[1] = 0x00;
                    payload[4] = 0x9d;
                    payload[5] = 0x01;
                    payload[6] = 0x2a;
                }
            }
            buffer
                .get_mut()
                .unwrap()
                .set_pts(gst::ClockTime::from_nseconds(0));
            src.push_buffer(buffer).unwrap();
        }
        // Wait for the burst to back up inside intake before releasing downstream.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while intake.property::<u32>("current-level-buffers") < 39
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(intake.property::<u32>("current-level-buffers") >= 39);
        drop(block);
        src.end_of_stream().unwrap();

        let assembled_bytes = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("complete 40-packet VP8 keyframe must survive intake queue");
        assert!(assembled_bytes > 3500);
        pipeline.set_state(gst::State::Null).unwrap();
    }
}
