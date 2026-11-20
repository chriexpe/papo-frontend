//! Receives SDP-identified SFU slots. SDK routes alone select a participant's output. Route changes
//! discard the old decoder and gate the replacement near its RTP sequence boundary.

use crate::{
    camera::android_codecs,
    diagnostics,
    negotiation::Codec,
    pipeline::{Branch, MediaResult, PadProbe, RequestedPad, element, filter, queue, ready_codec},
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as app;
use gstreamer_rtp as rtp;
use gstreamer_video as video;
use papo_sdk::{MediaFatalError, VideoDestination, VideoDestinationId, VideoSlotBinding};
use std::{
    any::Any,
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU32, Ordering},
    },
};

/// Owned NV12 frame. Offsets and strides are in bytes and describe the actual buffer layout.
pub struct CpuVideoFrame {
    /// Width in pixels.
    pub width_px: u32,
    /// Height in pixels.
    pub height_px: u32,
    /// Byte offsets for the Y and UV planes.
    pub offsets: [usize; 2],
    /// Row strides for the Y and UV planes.
    pub strides: [i32; 2],
    /// GStreamer colorimetry, including range. The CPU path preserves source orientation.
    pub colorimetry: video::VideoColorimetry,
    /// Frame bytes including any per-plane padding.
    pub bytes: Vec<u8>,
}

/// Provisional single-frame, latest-wins destination. Allocate one per SDK binding.
pub struct CpuVideoDestination {
    id: VideoDestinationId,
    mailbox: Mutex<Mailbox>,
}

#[derive(Default)]
struct Mailbox {
    generation: u64,
    revoked: bool,
    frame: Option<CpuVideoFrame>,
}

impl CpuVideoDestination {
    /// Creates an empty destination with the host's stable unique binding ID.
    pub fn new(id: VideoDestinationId) -> Self {
        Self {
            id,
            mailbox: Mutex::default(),
        }
    }

    /// Takes the most recent frame, clearing the pending slot.
    pub fn take(&self) -> Option<CpuVideoFrame> {
        self.lock().frame.take()
    }

    fn lock(&self) -> MutexGuard<'_, Mailbox> {
        self.mailbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn reset(&self) -> u64 {
        let mut mailbox = self.lock();
        mailbox.frame = None;
        mailbox.generation += 1;
        mailbox.generation
    }

    fn put(&self, generation: u64, sample: &gst::Sample) {
        let Some(frame) = cpu_frame(sample) else {
            return;
        };
        let mut mailbox = self.lock();
        if !mailbox.revoked && mailbox.generation == generation {
            mailbox.frame = Some(frame);
        }
    }
}

impl VideoDestination for CpuVideoDestination {
    fn id(&self) -> VideoDestinationId {
        self.id
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }

    fn revoke(&self) {
        let mut mailbox = self.lock();
        mailbox.revoked = true;
        mailbox.frame = None;
    }
}

fn cpu_frame(sample: &gst::Sample) -> Option<CpuVideoFrame> {
    use video::VideoFrameExt as _;
    let info = video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let frame = video::VideoFrameRef::from_buffer_ref_readable(sample.buffer()?, &info).ok()?;
    let stride = frame.plane_stride();
    let y = frame.plane_data(0).ok()?;
    let uv = frame.plane_data(1).ok()?;
    // Avoid a per-pixel conversion and preserve the mapped plane strides. Mapping validates bounds.
    let mut bytes = Vec::with_capacity(y.len().checked_add(uv.len())?);
    bytes.extend_from_slice(y);
    bytes.extend_from_slice(uv);
    Some(CpuVideoFrame {
        width_px: info.width(),
        height_px: info.height(),
        offsets: [0, y.len()],
        strides: [*stride.first()?, *stride.get(1)?],
        colorimetry: info.colorimetry(),
        bytes,
    })
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
    gate: Arc<SequenceGate>,
    wire_pad: gst::Pad,
    _wire_probe: PadProbe,
}

impl ReceiveVideo {
    pub fn attach(
        &mut self,
        graph: &gst::Pipeline,
        pad: &gst::Pad,
        track: String,
        codec: Codec,
    ) -> MediaResult<()> {
        let binding = self.plan.get(&track);
        let gate = Arc::new(SequenceGate::new(binding.and_then(|route| route.first_seq)));
        let tee = element("tee")?;
        tee.set_property("allow-not-linked", true);
        let inlet = Branch::attach(graph, vec![tee.clone()], Some(pad), None)?;
        let output = RequestedPad::new(&tee, "src_%u")?;
        let wire = wire_sequence_pad(pad);
        if wire != *pad && binding.is_some_and(|route| route.first_seq.is_some()) {
            // The pad-added fence may already be holding one resequenced buffer.
            // Only the buffers examined at the wire pad can enter this new route.
            PadProbe::discard_one(pad);
        }
        let wire_probe = gate.install(&wire);
        let branch = branch(graph, &output.pad, codec, binding)?;
        self.ports.insert(
            pad.clone(),
            Port {
                track,
                binding: binding.cloned(),
                branch,
                output,
                _inlet: inlet,
                gate,
                wire_pad: wire,
                _wire_probe: wire_probe,
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
        self.plan = bindings
            .into_iter()
            .map(|entry| (entry.track_id.clone(), entry))
            .collect();
        let Some(codec) = codec else { return Ok(()) };
        // Some destinations move to a different slot in this same snapshot. Invalidate
        // every old generation before installing any new branch for those destinations.
        for port in self.ports.values() {
            if !equivalent(port.binding.as_ref(), self.plan.get(&port.track)) {
                clear(port.binding.as_ref());
            }
        }
        for port in self.ports.values_mut() {
            let next = self.plan.get(&port.track);
            if equivalent(port.binding.as_ref(), next) {
                continue;
            }
            let _idle = PadProbe::idle(&port.wire_pad).await?;
            port.branch = None;
            if route_identity(port.binding.as_ref()) != route_identity(next) {
                port.gate.arm(next.and_then(|route| route.first_seq));
            }
            port.branch = branch(graph, &port.output.pad, codec, next)?;
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
        self.ports.clear();
        self.plan.clear();
    }
}

// webrtcbin may expose an rtpbin pad backed by rtpulpfecdec. That element
// resequences packets even in passthrough. first_seq is in SFU wire sequence space.
fn wire_sequence_pad(exposed: &gst::Pad) -> gst::Pad {
    let mut pad = exposed.clone();
    while let Some(target) = pad
        .downcast_ref::<gst::GhostPad>()
        .and_then(gst::GhostPad::target)
    {
        pad = target;
    }
    pad.parent_element()
        .filter(|element| {
            element
                .factory()
                .is_some_and(|factory| factory.name() == "rtpulpfecdec")
        })
        .and_then(|element| element.static_pad("sink"))
        .unwrap_or_else(|| exposed.clone())
}

fn route_identity(
    route: Option<&VideoSlotBinding>,
) -> Option<(papo_sdk::Uuid, papo_sdk::VideoKind, Option<u16>)> {
    route.map(|route| (route.participant, route.kind, route.first_seq))
}

fn equivalent(previous: Option<&VideoSlotBinding>, next: Option<&VideoSlotBinding>) -> bool {
    let destination = |route: Option<&VideoSlotBinding>| {
        route.and_then(|route| {
            route
                .destination
                .as_ref()
                .map(|destination| destination.id())
        })
    };
    route_identity(previous) == route_identity(next) && destination(previous) == destination(next)
}

fn cpu_destination(destination: &dyn VideoDestination) -> Option<&CpuVideoDestination> {
    destination.as_any().downcast_ref()
}

fn clear(binding: Option<&VideoSlotBinding>) {
    if let Some(destination) = binding
        .and_then(|route| route.destination.as_deref())
        .and_then(cpu_destination)
    {
        destination.reset();
    }
}

fn branch(
    graph: &gst::Pipeline,
    source: &gst::Pad,
    codec: Codec,
    binding: Option<&VideoSlotBinding>,
) -> MediaResult<Option<Branch>> {
    let destination = binding
        .and_then(|route| route.destination.as_ref())
        .and_then(|destination| {
            cpu_destination(destination.as_ref()).map(|cpu| (Arc::clone(destination), cpu.reset()))
        });
    let Some((destination, generation)) = destination else {
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
                        cpu.put(generation, &sample);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        )
        .build();
    let mut elements = vec![queue(8, 0)?, depay];
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

struct SequenceGate(AtomicU32);

impl SequenceGate {
    fn new(first: Option<u16>) -> Self {
        Self(AtomicU32::new(first.map_or(u32::MAX, u32::from)))
    }

    fn arm(&self, first: Option<u16>) {
        self.0
            .store(first.map_or(u32::MAX, u32::from), Ordering::Relaxed);
    }

    fn install(self: &Arc<Self>, pad: &gst::Pad) -> PadProbe {
        let gate = Arc::clone(self);
        PadProbe::data(pad, move |_, info| {
            if gate.0.load(Ordering::Relaxed) == u32::MAX {
                return gst::PadProbeReturn::Ok;
            }
            let accepts = |buffer: &gst::BufferRef| {
                rtp::RTPBuffer::from_buffer_readable(buffer)
                    .is_ok_and(|packet| gate.accept(packet.seq()))
            };
            let ok = info.buffer().is_some_and(|buffer| accepts(buffer))
                || info
                    .buffer_list()
                    .is_some_and(|list| list.iter().all(accepts));
            if ok {
                gst::PadProbeReturn::Ok
            } else {
                gst::PadProbeReturn::Drop
            }
        })
    }

    fn accept(&self, seq: u16) -> bool {
        let Ok(first) = u16::try_from(self.0.load(Ordering::Relaxed)) else {
            return true;
        };
        let distance = seq.wrapping_sub(first);
        if distance >= 0x8000 {
            return false;
        }
        // Retain a short window past the switch for late retransmissions, then
        // retire the comparison before the sequence space can wrap again.
        const SWITCH_WINDOW_PACKETS: u16 = 128;
        if distance >= SWITCH_WINDOW_PACKETS {
            let _ = self.0.compare_exchange(
                u32::from(first),
                u32::MAX,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sequence_gate_retires_after_crossing_the_wraparound_boundary() {
        let gate = SequenceGate::new(Some(1));
        assert!(!gate.accept(u16::MAX));
        assert!(!gate.accept(0));
        assert!(gate.accept(1));
        assert!(!gate.accept(u16::MAX));
        assert!(gate.accept(129));
        assert!(gate.accept(33_000));
        assert!(gate.accept(0));
    }
}
