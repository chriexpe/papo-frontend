//! Reading media without playing it: the pipeline paused on its first frame (no sound decoded,
//! no audio device opened), its length, its size, whether it has sound, then let go. Blocking:
//! the caller runs it off the UI thread.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;

use super::frames::FrameRelay;
use super::gst_engine::{failure_of, init};
use super::pipeline::{self, Mode, UNSUPPORTED};
use super::{MediaInfo, PlaybackFailure, Probe};

/// How long a probe waits for the first frame before it calls the media broken.
const PROBE_DEADLINE: Duration = Duration::from_secs(15);

/// Probes `bytes`, its first frame fitted into `poster_px`.
pub(super) fn probe(bytes: Arc<[u8]>, poster_px: [u32; 2]) -> Result<Probe, PlaybackFailure> {
    init()?;
    let graph = pipeline::build(bytes, Mode::Probe, Some(poster_px), None)?;
    let outcome = preroll(&graph);
    let probe = outcome.map(|()| {
        let duration_ms = graph
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(gst::ClockTime::mseconds);
        let poster = graph
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .and_then(|appsink| appsink.pull_preroll().ok())
            .and_then(|sample| FrameRelay::new(Arc::new(NoSink)).pack(&sample));
        Probe {
            info: MediaInfo {
                duration_ms,
                video_px: *graph
                    .video_px
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
                has_audio: *graph
                    .has_audio
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            },
            poster,
        }
    });
    let _ = graph.pipeline.set_state(gst::State::Null);
    probe
}

/// Pauses the pipeline and waits until it holds its first frame, or fails.
fn preroll(graph: &pipeline::Graph) -> Result<(), PlaybackFailure> {
    let bus = graph.pipeline.bus().ok_or(PlaybackFailure::Unavailable)?;
    if graph.pipeline.set_state(gst::State::Paused).is_err() {
        return Err(bus_failure(&bus).unwrap_or(PlaybackFailure::Broken));
    }
    let deadline = std::time::Instant::now() + PROBE_DEADLINE;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(PlaybackFailure::Broken);
        }
        let Some(message) = bus.timed_pop(gst::ClockTime::from_nseconds(
            u64::try_from(left.as_nanos()).unwrap_or(u64::MAX),
        )) else {
            return Err(PlaybackFailure::Broken);
        };
        match message.view() {
            gst::MessageView::AsyncDone(_) => return Ok(()),
            gst::MessageView::Error(error) => return Err(failure_of(error)),
            gst::MessageView::Application(application)
                if application
                    .structure()
                    .is_some_and(|structure| structure.name() == UNSUPPORTED) =>
            {
                return Err(PlaybackFailure::Unsupported);
            }
            _ => {}
        }
    }
}

/// A probe packs its one frame itself: nothing is sent anywhere.
struct NoSink;

impl super::FrameSink for NoSink {
    fn frame(&self, _frame: super::PackedNv12) {}
}

/// The failure an error already on `bus` states, if any.
fn bus_failure(bus: &gst::Bus) -> Option<PlaybackFailure> {
    bus.pop_filtered(&[gst::MessageType::Error])
        .and_then(|message| match message.view() {
            gst::MessageView::Error(error) => Some(failure_of(error)),
            _ => None,
        })
}
