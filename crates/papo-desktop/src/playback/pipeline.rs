//! The pipeline media plays (or is probed) through: bytes into `decodebin`, which is told to
//! skip every hardware decoder, so no VA decoder's frames are downloaded to system memory (the
//! costliest path; `docs/papo-desktop.md` §7, D7c "Frames"); a stream nothing here decodes makes
//! the media unsupported. Video is converted to NV12 (never RGB: Rune converts in its shader) and
//! scaled to the frame size before an appsink holding one buffer, dropping late ones; sound goes
//! through a volume to the system's audio sink. Running the pipeline is `gst_engine`'s and
//! `probe`'s.

use std::sync::{Arc, Mutex, PoisonError};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use super::frames::FrameRelay;
use super::source::byte_source;
use super::{FrameSink, PlaybackFailure, fitted_frame};

/// The most threads a software decoder runs (libav's default is one per core).
const DECODER_THREADS: i32 = 4;

/// Decoded frames waiting between the decoder and conversion.
const RAW_FRAMES_QUEUED: u32 = 3;

/// The application message a pipeline posts when a stream has no decoder here.
pub(super) const UNSUPPORTED: &str = "papo-unsupported";

/// What a pipeline is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// Plays: sound to the audio device, frames on time.
    Play,
    /// Reads the first frame and the length: no sound decoded, frames as soon as decoded.
    Probe,
}

/// The volume a player asked for, and its element once the sound's stream appears.
#[derive(Default)]
pub(super) struct Loudness {
    muted: bool,
    volume: f64,
    element: Option<gst::Element>,
}

impl Loudness {
    fn apply(&self) {
        if let Some(element) = &self.element {
            element.set_property("mute", self.muted);
            element.set_property("volume", self.volume);
        }
    }

    /// Sets the sound to `muted` at `volume` (0 to 1), now or when its stream appears.
    pub(super) fn set(&mut self, muted: bool, volume: f32) {
        self.muted = muted;
        self.volume = f64::from(volume.clamp(0.0, 1.0));
        self.apply();
    }
}

/// A built pipeline and the parts of it its owner drives.
pub(super) struct Graph {
    pub(super) pipeline: gst::Pipeline,
    pub(super) loudness: Arc<Mutex<Loudness>>,
    /// The video's frame filter, to change the frame size while playing.
    pub(super) frame_caps: Arc<Mutex<Option<gst::Element>>>,
    /// The video's appsink (probe: to pull its first frame).
    pub(super) frames: Arc<Mutex<Option<gst_app::AppSink>>>,
    /// Whether a sound stream appeared.
    pub(super) has_audio: Arc<Mutex<bool>>,
    /// The video's displayed size, once its stream appeared.
    pub(super) video_px: Arc<Mutex<Option<[u32; 2]>>>,
    /// The video's frames on their way to the sink, once its stream appeared.
    pub(super) relay: Arc<Mutex<Option<Arc<FrameRelay>>>>,
}

/// NV12 frames at most `frame_px`, or at the stream's size.
pub(super) fn frame_caps(frame_px: Option<[u32; 2]>) -> gst::Caps {
    let caps = gst::Caps::builder("video/x-raw").field("format", "NV12");
    match frame_px {
        Some([width, height]) => caps
            .field("width", i32::try_from(width).unwrap_or(i32::MAX))
            .field("height", i32::try_from(height).unwrap_or(i32::MAX))
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build(),
        None => caps.build(),
    }
}

/// Builds the pipeline for `bytes`, its frames fitted into `box_px` (device pixels; `None`: the
/// stream's own size). Its streams' branches are added as `parsebin` finds them.
///
/// # Errors
/// An element GStreamer lacks (its base plugins missing).
pub(super) fn build(
    bytes: Arc<[u8]>,
    mode: Mode,
    box_px: Option<[u32; 2]>,
    sink: Option<Arc<dyn FrameSink>>,
) -> Result<Graph, PlaybackFailure> {
    let pipeline = gst::Pipeline::new();
    let source = byte_source(bytes);
    let decode = make("decodebin")?;
    pipeline
        .add_many([source.upcast_ref(), &decode])
        .map_err(|_| PlaybackFailure::Unavailable)?;
    source
        .link(&decode)
        .map_err(|_| PlaybackFailure::Unavailable)?;
    software_only(&decode)?;
    let Ok(decode_bin) = decode.clone().downcast::<gst::Bin>() else {
        return Err(PlaybackFailure::Unavailable);
    };
    decode_bin.connect_element_added(|_, element| {
        // A frame-threaded decoder starts a thread per core, each holding frames of its own:
        // a few are enough for a message's video, and keep its memory bounded.
        let threads = element
            .find_property("max-threads")
            .is_some_and(|spec| spec.value_type() == i32::static_type());
        if threads {
            element.set_property("max-threads", DECODER_THREADS);
        }
    });
    let unknown = pipeline.downgrade();
    decode.connect("unknown-type", false, move |_| {
        // A stream nothing here decodes: the media is unsupported as a whole, so Save is
        // offered rather than silence or a still picture.
        if let Some(pipeline) = unknown.upgrade() {
            let message = gst::message::Application::new(gst::Structure::new_empty(UNSUPPORTED));
            let _ = pipeline.post_message(message);
        }
        None
    });
    let graph = Graph {
        pipeline: pipeline.clone(),
        loudness: Arc::new(Mutex::new(Loudness {
            volume: 1.0,
            ..Loudness::default()
        })),
        frame_caps: Arc::default(),
        frames: Arc::default(),
        has_audio: Arc::default(),
        video_px: Arc::default(),
        relay: Arc::default(),
    };
    let branches = Branches {
        pipeline: pipeline.downgrade(),
        mode,
        box_px,
        sink,
        loudness: Arc::clone(&graph.loudness),
        frame_caps: Arc::clone(&graph.frame_caps),
        frames: Arc::clone(&graph.frames),
        has_audio: Arc::clone(&graph.has_audio),
        video_px: Arc::clone(&graph.video_px),
        relay: Arc::clone(&graph.relay),
    };
    decode.connect_pad_added(move |_, pad| branches.add(pad));
    Ok(graph)
}

/// What a new stream's branch needs.
struct Branches {
    pipeline: gst::glib::WeakRef<gst::Pipeline>,
    mode: Mode,
    box_px: Option<[u32; 2]>,
    sink: Option<Arc<dyn FrameSink>>,
    loudness: Arc<Mutex<Loudness>>,
    frame_caps: Arc<Mutex<Option<gst::Element>>>,
    frames: Arc<Mutex<Option<gst_app::AppSink>>>,
    has_audio: Arc<Mutex<bool>>,
    video_px: Arc<Mutex<Option<[u32; 2]>>>,
    relay: Arc<Mutex<Option<Arc<FrameRelay>>>>,
}

impl Branches {
    fn add(&self, pad: &gst::Pad) {
        let Some(pipeline) = self.pipeline.upgrade() else {
            return;
        };
        let Some(caps) = pad.current_caps().or_else(|| pad.allowed_caps()) else {
            return;
        };
        let Some(kind) = caps.structure(0).map(|s| s.name().to_string()) else {
            return;
        };
        let built = if kind.starts_with("video/") {
            self.video(&caps)
        } else if kind.starts_with("audio/") {
            *self
                .has_audio
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = true;
            self.audio()
        } else {
            // Cover art, subtitles, data: nothing Papo shows (queued, as sound is).
            discard()
        };
        let elements = built.or_else(|_| discard()).unwrap_or_default();
        attach(&pipeline, pad, &elements);
    }

    fn video(&self, caps: &gst::Caps) -> Result<Vec<gst::Element>, PlaybackFailure> {
        // Decoded frames are large (3 MiB at 1080p): the queue that decouples the decoder from
        // conversion holds a few, never a second's worth.
        let frames = make("queue")?;
        frames.set_property("max-size-buffers", RAW_FRAMES_QUEUED);
        frames.set_property("max-size-bytes", 0_u32);
        frames.set_property("max-size-time", 0_u64);
        let mut chain = vec![frames, make("videoconvert")?];
        chain.push(make("videoscale")?);
        let source_px = displayed_px(caps);
        *self.video_px.lock().unwrap_or_else(PoisonError::into_inner) = source_px;
        // Fitted into the box, or at the stream's own size: either way with even sides, as
        // NV12 frames must have.
        let frame_px = source_px.map(|source| fitted_frame(source, self.box_px.unwrap_or(source)));
        let filter = make("capsfilter")?;
        filter.set_property("caps", frame_caps(frame_px));
        *self
            .frame_caps
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(filter.clone());
        chain.push(filter);
        let appsink = gst_app::AppSink::builder()
            .max_buffers(1)
            .drop(true)
            .sync(self.mode == Mode::Play)
            .qos(self.mode == Mode::Play)
            .enable_last_sample(false)
            .build();
        if let Some(sink) = &self.sink {
            let relay = FrameRelay::new(Arc::clone(sink));
            *self.relay.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&relay));
            let preroll_relay = Arc::clone(&relay);
            appsink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_preroll(move |appsink| {
                        if let Ok(sample) = appsink.pull_preroll() {
                            preroll_relay.offer(sample);
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .new_sample(move |appsink| {
                        let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        relay.offer(sample);
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
        }
        *self.frames.lock().unwrap_or_else(PoisonError::into_inner) = Some(appsink.clone());
        chain.push(appsink.upcast());
        Ok(chain)
    }

    fn audio(&self) -> Result<Vec<gst::Element>, PlaybackFailure> {
        if self.mode == Mode::Probe {
            // Its own queue: a sink waiting for its first buffer must not hold the decoder
            // back from the video's.
            return discard();
        }
        let mut chain = vec![make("queue")?, make("audioconvert")?];
        chain.push(make("audioresample")?);
        let volume = make("volume")?;
        {
            let mut loudness = self.loudness.lock().unwrap_or_else(PoisonError::into_inner);
            loudness.element = Some(volume.clone());
            loudness.apply();
        }
        chain.push(volume);
        chain.push(make("autoaudiosink")?);
        Ok(chain)
    }
}

/// A branch that takes a stream and drops it, behind its own queue.
fn discard() -> Result<Vec<gst::Element>, PlaybackFailure> {
    Ok(vec![make("queue")?, make("fakesink")?])
}

/// Adds `elements` to `pipeline` in a chain from `pad`, started.
fn attach(pipeline: &gst::Pipeline, pad: &gst::Pad, elements: &[gst::Element]) {
    let Some(first) = elements.first() else {
        return;
    };
    if pipeline.add_many(elements).is_err() || gst::Element::link_many(elements).is_err() {
        log::warn!("Couldn't build a media branch");
        return;
    }
    for element in elements {
        let _ = element.sync_state_with_parent();
    }
    if let Some(sink_pad) = first.static_pad("sink") {
        let _ = pad.link(&sink_pad);
    }
}

/// The size a video stream of `caps` is displayed at: its pixels, widened or narrowed by their
/// aspect.
pub(super) fn displayed_px(caps: &gst::Caps) -> Option<[u32; 2]> {
    let structure = caps.structure(0)?;
    let width = u32::try_from(structure.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(structure.get::<i32>("height").ok()?).ok()?;
    let aspect = structure
        .get::<gst::Fraction>("pixel-aspect-ratio")
        .ok()
        .filter(|aspect| aspect.numer() > 0 && aspect.denom() > 0);
    let width = match aspect {
        Some(aspect) => {
            let widened = u64::from(width) * u64::try_from(aspect.numer()).ok()?
                / u64::try_from(aspect.denom()).ok()?;
            u32::try_from(widened).ok()?
        }
        None => width,
    };
    (width > 0 && height > 0).then_some([width, height])
}

/// A new element of `factory`.
fn make(factory: &str) -> Result<gst::Element, PlaybackFailure> {
    gst::ElementFactory::make(factory).build().map_err(|_| {
        log::warn!("GStreamer element unavailable: {factory}");
        PlaybackFailure::Unavailable
    })
}

/// Tells `decode` to skip hardware decoders whenever it picks one.
///
/// # Errors
/// [`PlaybackFailure::Unavailable`] when `decodebin`'s choice can't be answered (its result
/// type isn't registered).
fn software_only(decode: &gst::Element) -> Result<(), PlaybackFailure> {
    // decodebin's own enum (GstAutoplugSelectResult): 0 tries the factory, 2 skips it.
    let results = gst::glib::Type::from_name("GstAutoplugSelectResult")
        .filter(|results| gst::glib::EnumClass::with_type(*results).is_some())
        .ok_or(PlaybackFailure::Unavailable)?;
    decode.connect("autoplug-select", false, move |values| {
        let factory = values
            .get(3)
            .and_then(|value| value.get::<gst::ElementFactory>().ok());
        let hardware = factory.as_ref().is_some_and(is_hardware);
        if let Some(factory) = factory.filter(|_| !hardware)
            && factory
                .metadata(gst::ELEMENT_METADATA_KLASS)
                .is_some_and(|klass| klass.contains("Decoder"))
        {
            log::debug!("Media decoder: {}", factory.name());
        }
        gst::glib::EnumClass::with_type(results)
            .and_then(|class| class.to_value(if hardware { 2 } else { 0 }))
    });
    Ok(())
}

/// Whether `factory` decodes on a GPU or other hardware: its frames would live there.
fn is_hardware(factory: &gst::ElementFactory) -> bool {
    factory
        .metadata(gst::ELEMENT_METADATA_KLASS)
        .is_some_and(|klass| klass.split('/').any(|part| part == "Hardware"))
}
