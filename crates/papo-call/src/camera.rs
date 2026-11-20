//! Camera capture is separable from the negotiated send line, so camera-off releases the device.

use crate::{
    diagnostics,
    negotiation::{Codec, sender},
    pipeline::{Branch, MediaResult, element, filter, queue},
    platform::MediaDevices,
};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as app;
use gstreamer_webrtc as rtc;
use papo_sdk::{CameraFacing, CaptureState, MediaFatalError};

const WIDTH_PX: i32 = 640;
const HEIGHT_PX: i32 = 360;
const BITRATE_BPS: u32 = 600_000;

#[derive(Default)]
pub(crate) struct Camera {
    capture: Option<Branch>,
    line: Option<SendLine>,
}

struct SendLine {
    transceiver: rtc::WebRTCRTPTransceiver,
    input: app::AppSrc,
    rtp: gst::Pad,
    branch: Branch,
}

impl Camera {
    pub fn rtp(&self) -> Option<&gst::Pad> {
        self.line.as_ref().map(|line| &line.rtp)
    }

    pub fn publishing(&self) -> bool {
        self.capture.is_some()
    }

    pub fn set(
        &mut self,
        graph: &gst::Pipeline,
        peer: &gst::Element,
        codec: Codec,
        device: Option<&dyn MediaDevices>,
        facing: CameraFacing,
    ) -> (CaptureState, bool) {
        self.release();
        let was_active = self.line.as_ref().is_some_and(|line| {
            line.transceiver.direction() == rtc::WebRTCRTPTransceiverDirection::Sendonly
        });
        let available = if let Some(devices) = device {
            self.start(graph, peer, codec, devices, facing).is_ok()
        } else {
            false
        };
        if let Some(line) = &self.line {
            line.transceiver.set_direction(if available {
                rtc::WebRTCRTPTransceiverDirection::Sendonly
            } else {
                rtc::WebRTCRTPTransceiverDirection::Inactive
            });
        }
        let state = if available {
            CaptureState::Active
        } else if device.is_some() {
            CaptureState::Unavailable
        } else {
            CaptureState::Inactive
        };
        (state, was_active != available)
    }

    fn start(
        &mut self,
        graph: &gst::Pipeline,
        peer: &gst::Element,
        codec: Codec,
        devices: &dyn MediaDevices,
        facing: CameraFacing,
    ) -> MediaResult<()> {
        let device = devices.camera(facing).ok_or(MediaFatalError::DeviceError)?;
        if self.line.is_none() {
            self.line = Some(SendLine::new(graph, peer, codec)?);
        }
        let line = self.line.as_ref().ok_or(MediaFatalError::Internal)?;
        let input = line.input.clone();
        let sink = app::AppSink::builder()
            .sync(false)
            .async_(false)
            .max_buffers(1)
            .drop(true)
            .callbacks(
                app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Flushing)?;
                        let mut buffer = sample.buffer_owned().ok_or(gst::FlowError::Error)?;
                        let writable = buffer.make_mut();
                        writable.set_pts(None);
                        writable.set_dts(None);
                        input.push_buffer(buffer)
                    })
                    .build(),
            )
            .build();
        self.capture = Some(Branch::attach(
            graph,
            vec![
                device,
                queue(2, 0)?,
                element("videoconvert")?,
                element("videoscale")?,
                element("videorate")?,
                filter(&raw_caps())?,
                sink.upcast(),
            ],
            None,
            None,
        )?);
        Ok(())
    }

    pub fn owns_capture(&self, source: &gst::Object) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|branch| branch.owns(source))
            || self
                .line
                .as_ref()
                .is_some_and(|line| line.branch.owns(source))
    }

    pub fn release(&mut self) {
        self.capture = None;
    }
}

impl SendLine {
    fn new(graph: &gst::Pipeline, peer: &gst::Element, codec: Codec) -> MediaResult<Self> {
        let input = app::AppSrc::builder()
            .caps(&raw_caps())
            .format(gst::Format::Time)
            .is_live(true)
            .do_timestamp(true)
            .max_buffers(2)
            .leaky_type(app::AppLeakyType::Downstream)
            .build();
        let caps = codec.caps();
        let (target, transceiver) = sender(peer, &caps)?;
        let rtp = target.pad.clone();
        let mut elements = vec![input.clone().upcast(), queue(2, 0)?];
        let encoder = match codec {
            Codec::Vp8 => {
                let encoder = element("vp8enc")?;
                encoder.set_property("deadline", 1i64);
                encoder.set_property(
                    "target-bitrate",
                    i32::try_from(BITRATE_BPS).unwrap_or(600_000),
                );
                encoder.set_property("keyframe-max-dist", 30i32);
                let pay = element("rtpvp8pay")?;
                pay.set_property("pt", 96u32);
                pay.set_property_from_str("picture-id-mode", "15-bit");
                elements.extend([encoder.clone(), pay]);
                encoder
            }
            Codec::H264 => {
                let encoder = h264_encoder()?;
                let pay = element("rtph264pay")?;
                let advertised = element("capssetter")?;
                // The room advertises Main level 5.0 with level asymmetry. Encoders may
                // write a lower level and different constraint flags in their SPS.
                advertised.set_property("caps", &caps);
                pay.set_property("pt", 96u32);
                pay.set_property("config-interval", -1i32);
                pay.set_property_from_str("aggregate-mode", "zero-latency");
                elements.extend([
                    element("videoconvert")?,
                    encoder.clone(),
                    element("h264parse")?,
                    filter(
                        &gst::Caps::builder("video/x-h264")
                            .field("profile", "main")
                            .build(),
                    )?,
                    pay,
                    advertised,
                ]);
                encoder
            }
        };
        // Negotiated codec preferences supply the send line's format before the first frame.
        let branch = Branch::attach(graph, elements, None, Some(target))?;
        diagnostics::codec_selected(&encoder, "encoder");
        Ok(Self {
            transceiver,
            input,
            rtp,
            branch,
        })
    }
}

fn h264_encoder() -> MediaResult<gst::Element> {
    let main = gst::Caps::builder("video/x-h264")
        .field("profile", "main")
        .build();
    let names = [
        "vah264enc",
        "vaapih264enc",
        "vtenc_h264_hw",
        "mfh264enc",
        "d3d11h264enc",
    ];
    for name in names
        .into_iter()
        .map(str::to_owned)
        .chain(android_codecs("amcvidenc-"))
    {
        if let Some(encoder) = crate::pipeline::ready_codec(&name, "src", &main) {
            return Ok(encoder);
        }
    }
    let encoder = element("openh264enc")?;
    encoder.set_property("bitrate", BITRATE_BPS);
    encoder.set_property("gop-size", 30u32);
    Ok(encoder)
}

pub(crate) fn android_codecs(prefix: &str) -> impl Iterator<Item = String> {
    gst::Registry::get()
        .features(gst::ElementFactory::static_type())
        .into_iter()
        .filter_map(move |feature| {
            let name = feature.name();
            (name.starts_with(prefix) && name.contains("avc") && feature.rank() != gst::Rank::NONE)
                .then(|| name.to_string())
        })
}

fn raw_caps() -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", WIDTH_PX)
        .field("height", HEIGHT_PX)
        .field("framerate", gst::Fraction::new(30, 1))
        .build()
}
