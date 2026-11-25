//! Decoded NV12 samples into `papo-call`'s packed frames, handed to each player's sink.
//!
//! Every player's frames are packed on one thread of the process (`papo-packer`): the planes are
//! copied once, straight from GStreamer's mapped buffer, by `papo-call`'s packing. One thread
//! keeps the allocator to one arena for all frames: packed on each player's streaming thread and
//! freed on Rune's, they left an arena per thread holding freed frames (about 150 MiB with three
//! videos playing). Each player keeps only its newest unpacked sample, so a packer that falls
//! behind drops stale frames instead of queueing them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use gstreamer as gst;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use papo_call::{Nv12Planes, PackedNv12, VideoColour};

use super::FrameSink;

/// What a stream's caps say, read once until they change.
struct Format {
    caps: gst::Caps,
    info: gst_video::VideoInfo,
    colour: VideoColour,
}

/// One player's frames on their way to its sink.
pub(super) struct FrameRelay {
    /// The newest sample not packed yet.
    latest: Mutex<Option<gst::Sample>>,
    /// The relay waits on the packer's queue.
    queued: AtomicBool,
    /// Its pipeline was let go: a frame still on its way is dropped, so it never lands after
    /// the next player's on the same surface.
    closed: AtomicBool,
    format: Mutex<Option<Format>>,
    sink: Arc<dyn FrameSink>,
}

impl FrameRelay {
    /// A relay of frames to `sink`.
    pub(super) fn new(sink: Arc<dyn FrameSink>) -> Arc<FrameRelay> {
        Arc::new(FrameRelay {
            latest: Mutex::new(None),
            queued: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            format: Mutex::new(None),
            sink,
        })
    }

    /// Its pipeline was let go: no frame reaches the sink any more.
    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// `sample` is the newest frame: it replaces one not packed yet, and the packer is told.
    pub(super) fn offer(self: &Arc<Self>, sample: gst::Sample) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(sample);
        if !self.queued.swap(true, Ordering::AcqRel) {
            packer().send(Arc::clone(self));
        }
    }

    /// Packs the newest sample, if any, and hands it to the sink (on the packer's thread).
    fn pack_latest(&self) {
        self.queued.store(false, Ordering::Release);
        let sample = self
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(frame) = sample.and_then(|sample| self.pack(&sample))
            && !self.closed.load(Ordering::Acquire)
        {
            self.sink.frame(frame);
        }
    }

    /// `sample` (NV12) as a packed frame; `None` for a sample without NV12 planes.
    pub(super) fn pack(&self, sample: &gst::Sample) -> Option<PackedNv12> {
        let caps = sample.caps()?;
        let buffer = sample.buffer()?;
        let mut format = self.format.lock().unwrap_or_else(PoisonError::into_inner);
        if format
            .as_ref()
            .is_none_or(|known| known.caps.as_ref() != caps)
        {
            let info = gst_video::VideoInfo::from_caps(caps).ok()?;
            let colour = VideoColour::from_caps_string(&info.colorimetry().to_string());
            *format = Some(Format {
                caps: caps.to_owned(),
                info,
                colour,
            });
        }
        let format = format.as_ref()?;
        let mapped =
            gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &format.info).ok()?;
        let strides = mapped.plane_stride();
        let (Some(&y_stride), Some(&uv_stride)) = (strides.first(), strides.get(1)) else {
            return None;
        };
        let (Ok(y), Ok(uv)) = (mapped.plane_data(0), mapped.plane_data(1)) else {
            return None;
        };
        Nv12Planes {
            size_px: [format.info.width(), format.info.height()],
            planes: [y, uv],
            strides: [y_stride, uv_stride],
            colour: format.colour,
        }
        .pack()
        .ok()
    }
}

/// The packer thread's queue: relays with a sample waiting, each at most once.
struct Packer {
    queue: Mutex<mpsc::Sender<Arc<FrameRelay>>>,
}

impl Packer {
    fn send(&self, relay: Arc<FrameRelay>) {
        let sent = self
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(relay);
        if sent.is_err() {
            log::warn!("The video frame packer stopped");
        }
    }
}

/// The process's packer, started the first time a frame comes.
fn packer() -> &'static Packer {
    static PACKER: OnceLock<Packer> = OnceLock::new();
    PACKER.get_or_init(|| {
        let (queue, inbox) = mpsc::channel::<Arc<FrameRelay>>();
        let started = std::thread::Builder::new()
            .name("papo-packer".into())
            .spawn(move || {
                for relay in inbox {
                    relay.pack_latest();
                }
            });
        if started.is_err() {
            log::warn!("The video frame packer couldn't start");
        }
        Packer {
            queue: Mutex::new(queue),
        }
    })
}
