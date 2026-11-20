//! Owns the permanent send-only Opus line and the microphone source opened only while unmuted.

use crate::{
    negotiation::{opus_caps, sender},
    pipeline::{Branch, MediaResult, RequestedPad, element, filter, queue},
    platform::MediaDevices,
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_rtp as rtp;
use papo_sdk::{CaptureState, MediaFatalError};
use rtp::prelude::*;

const CAPTURE_QUEUE_NS: u64 = 80_000_000;

pub(crate) struct Mic {
    input: gst::Element,
    pub rtp: gst::Pad,
    capture: Option<Branch>,
    _line: Branch,
    _silence: Branch,
}

pub(crate) fn pcm_caps() -> gst::Caps {
    gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("layout", "interleaved")
        .field("rate", 48_000i32)
        .field("channels", 2i32)
        .build()
}

impl Mic {
    pub fn new(graph: &gst::Pipeline, peer: &gst::Element) -> MediaResult<Self> {
        let input = element("audiomixer")?;
        input.set_property("ignore-inactive-pads", true);
        let level = element("level")?;
        level.set_property("audio-level-meta", true);
        level.set_property("post-messages", false);
        level.set_property("interval", 20_000_000u64);
        let encoder = element("opusenc")?;
        encoder.set_property("bitrate", 48_000i32);
        encoder.set_property("inband-fec", true);
        encoder.set_property("packet-loss-percentage", 10i32);
        encoder.set_property_from_str("audio-type", "voice");
        let pay = element("rtpopuspay")?;
        pay.set_property("pt", 111u32);
        let extension =
            rtp::RTPHeaderExtension::create_from_uri("urn:ietf:params:rtp-hdrext:ssrc-audio-level")
                .ok_or(MediaFatalError::DeviceError)?;
        extension.set_id(9);
        pay.emit_by_name::<()>("add-extension", &[&extension]);
        let caps = opus_caps();
        let mut advertised = caps.clone();
        if let Some(fields) = advertised.make_mut().structure_mut(0) {
            fields.set(
                "extmap-9",
                gst::Array::new(["", "urn:ietf:params:rtp-hdrext:ssrc-audio-level", "vad=on"]),
            );
        }
        let (target, _) = sender(peer, &advertised)?;
        let rtp = target.pad.clone();
        let line = Branch::attach(
            graph,
            vec![
                input.clone(),
                filter(&pcm_caps())?,
                level,
                encoder,
                pay,
                filter(&caps)?,
            ],
            None,
            Some(target),
        )?;

        // A device-free source keeps the RTP caps, Opus timestamps and microphone m-line
        // stable while muted. Receivers can hear audio even if capture fails to open.
        let silence = element("audiotestsrc")?;
        silence.set_property("is-live", true);
        silence.set_property_from_str("wave", "silence");
        let silence = Branch::attach(
            graph,
            vec![silence, filter(&pcm_caps())?],
            None,
            Some(RequestedPad::new(&input, "sink_%u")?),
        )?;
        Ok(Self {
            input,
            rtp,
            capture: None,
            _line: line,
            _silence: silence,
        })
    }

    pub fn set_muted(
        &mut self,
        graph: &gst::Pipeline,
        devices: &dyn MediaDevices,
        muted: bool,
    ) -> CaptureState {
        if muted {
            self.capture = None;
            return CaptureState::Inactive;
        }
        if self.capture.is_none() {
            self.capture = self.open(graph, devices).ok();
        }
        if self.capture.is_some() {
            CaptureState::Active
        } else {
            CaptureState::Unavailable
        }
    }

    fn open(&self, graph: &gst::Pipeline, devices: &dyn MediaDevices) -> MediaResult<Branch> {
        let source = devices.microphone().ok_or(MediaFatalError::DeviceError)?;
        if source.has_property("preset") {
            source.set_property_from_str("preset", "voice-communication");
        }
        if source.has_property("do-timestamp") {
            source.set_property("do-timestamp", true);
        }
        Branch::attach(
            graph,
            vec![
                source,
                queue(0, CAPTURE_QUEUE_NS)?,
                element("audioconvert")?,
                element("audioresample")?,
                filter(&pcm_caps())?,
            ],
            None,
            Some(RequestedPad::new(&self.input, "sink_%u")?),
        )
    }

    pub fn owns_capture(&self, source: &gst::Object) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|branch| branch.owns(source))
    }

    pub fn release(&mut self) {
        self.capture = None;
    }
}
