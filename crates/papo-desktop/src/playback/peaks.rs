//! Reading audio for its waveform (`docs/design.md` §14.5; Open, "Waveform peaks"): the sound
//! decoded on a pipeline of its own, never the player's (a seek or a loop there would feed the
//! waveform out of order), to 32-bit floats as fast as it decodes, into the SDK's
//! [`WaveformBuilder`], which owns the reduction. Its length is read first: audio longer than the
//! read allows stops there. Blocking: the caller runs it off the UI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use papo_sdk_ui::{Waveform, WaveformBuilder};

use super::gst_engine::init;
use super::pipeline::{self, Mode, discard, make};
use super::probe::{bus_failure, preroll};
use super::{AudioRead, PlaybackFailure};

/// Decoded sound buffers the reader may fall behind by before the decoder waits.
const SAMPLES_QUEUED: u32 = 8;

/// How often the waveform so far is handed out while decoding.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// How long the reader waits for the next decoded buffer before it looks at the bus and at
/// `stop` again.
const PULL_WAIT: Duration = Duration::from_millis(100);

/// How long a decode may go without a buffer before the audio is called broken.
const STALL_DEADLINE: Duration = Duration::from_secs(15);

/// Reads `bytes`: see [`super::Engine::read_audio`].
pub(super) fn read_audio(
    bytes: Arc<[u8]>,
    ahead_max_ms: Option<u64>,
    progress: &(dyn Fn(Waveform) + Send + Sync),
    stop: &AtomicBool,
) -> Result<AudioRead, PlaybackFailure> {
    init()?;
    let graph = pipeline::build(bytes, Mode::Peaks, None, None)?;
    let read = read(&graph, ahead_max_ms, progress, stop);
    let _ = graph.pipeline.set_state(gst::State::Null);
    read
}

fn read(
    graph: &pipeline::Graph,
    ahead_max_ms: Option<u64>,
    progress: &(dyn Fn(Waveform) + Send + Sync),
    stop: &AtomicBool,
) -> Result<AudioRead, PlaybackFailure> {
    preroll(graph)?;
    let samples = graph
        .samples
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    // Media without sound has no waveform: it isn't audio.
    let samples = samples.ok_or(PlaybackFailure::Unsupported)?;
    let duration_ms = graph
        .pipeline
        .query_duration::<gst::ClockTime>()
        .map(gst::ClockTime::mseconds);
    let unread = AudioRead {
        duration_ms,
        waveform: None,
    };
    let too_long = ahead_max_ms
        .zip(duration_ms)
        .is_some_and(|(max_ms, duration_ms)| duration_ms > max_ms);
    if too_long || stop.load(Ordering::Relaxed) {
        return Ok(unread);
    }
    let bus = graph.pipeline.bus().ok_or(PlaybackFailure::Unavailable)?;
    if graph.pipeline.set_state(gst::State::Playing).is_err() {
        return Err(bus_failure(&bus).unwrap_or(PlaybackFailure::Broken));
    }
    let wait = gst::ClockTime::from_mseconds(u64::try_from(PULL_WAIT.as_millis()).unwrap_or(100));
    let mut decoding = Decoding::default();
    let mut progressed_at = Instant::now();
    let mut buffer_at = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(unread);
        }
        if let Some(failure) = bus_failure(&bus) {
            return Err(failure);
        }
        match samples.try_pull_sample(wait) {
            Some(sample) => {
                decoding.push(&sample, duration_ms);
                buffer_at = Instant::now();
                // Without a stated length the read is bounded by what it decoded.
                if ahead_max_ms.is_some_and(|max_ms| decoding.decoded_ms() > max_ms) {
                    return Ok(unread);
                }
                if progressed_at.elapsed() >= PROGRESS_EVERY
                    && let Some(builder) = &decoding.builder
                {
                    progress(builder.snapshot());
                    progressed_at = Instant::now();
                }
            }
            None if samples.is_eos() => break,
            None if buffer_at.elapsed() > STALL_DEADLINE => return Err(PlaybackFailure::Broken),
            None => {}
        }
    }
    let decoded_ms = decoding.decoded_ms();
    // A stream that ended before any sound was decoded is damaged.
    let builder = decoding.builder.ok_or(PlaybackFailure::Broken)?;
    Ok(AudioRead {
        duration_ms: duration_ms.or(Some(decoded_ms)),
        waveform: Some(builder.finish()),
    })
}

/// The decode so far: the builder starts with the first buffer, whose caps give the rate.
#[derive(Default)]
struct Decoding {
    builder: Option<WaveformBuilder>,
    /// Frames a second, from the first buffer's caps.
    rate: u32,
    /// Frames decoded so far.
    frames: u64,
    /// The current buffer's samples, reused.
    scratch: Vec<f32>,
}

impl Decoding {
    fn push(&mut self, sample: &gst::Sample, duration_ms: Option<u64>) {
        let Some((rate, channels)) = sample.caps().and_then(format) else {
            return;
        };
        if self.builder.is_none() {
            self.rate = rate;
            let expected = duration_ms.map(|duration_ms| duration_ms * u64::from(rate) / 1000);
            self.builder = Some(WaveformBuilder::new(expected));
        }
        let (Some(builder), Some(buffer)) = (self.builder.as_mut(), sample.buffer()) else {
            return;
        };
        let Ok(map) = buffer.map_readable() else {
            return;
        };
        // Copied out: a mapped buffer need not be aligned for f32.
        self.scratch.clear();
        let (samples, _) = map.as_slice().as_chunks::<4>();
        self.scratch
            .extend(samples.iter().map(|bytes| f32::from_le_bytes(*bytes)));
        builder.push(&self.scratch, channels);
        self.frames += (self.scratch.len() / usize::from(channels)) as u64;
    }

    fn decoded_ms(&self) -> u64 {
        if self.rate == 0 {
            return 0;
        }
        self.frames * 1000 / u64::from(self.rate)
    }
}

/// A raw sound buffer's rate and channel count, from its caps.
fn format(caps: &gst::CapsRef) -> Option<(u32, u16)> {
    let structure = caps.structure(0)?;
    let rate = u32::try_from(structure.get::<i32>("rate").ok()?).ok()?;
    let channels = u16::try_from(structure.get::<i32>("channels").ok()?).ok()?;
    (rate > 0 && channels > 0).then_some((rate, channels))
}

/// The branch a pipeline reading a waveform gives its first sound stream: decoded to
/// interleaved 32-bit floats (any rate, any channels) for the reader to pull from `samples`; a
/// later sound stream is discarded.
pub(super) fn samples_branch(
    samples: &Mutex<Option<gst_app::AppSink>>,
) -> Result<Vec<gst::Element>, PlaybackFailure> {
    let mut samples = samples.lock().unwrap_or_else(PoisonError::into_inner);
    if samples.is_some() {
        return discard();
    }
    let filter = make("capsfilter")?;
    filter.set_property(
        "caps",
        gst::Caps::builder("audio/x-raw")
            .field("format", "F32LE")
            .field("layout", "interleaved")
            .build(),
    );
    // Unsynced (as fast as it decodes), and bounded: the decoder waits for the reader.
    let appsink = gst_app::AppSink::builder()
        .sync(false)
        .max_buffers(SAMPLES_QUEUED)
        .drop(false)
        .enable_last_sample(false)
        .build();
    *samples = Some(appsink.clone());
    Ok(vec![
        make("queue")?,
        make("audioconvert")?,
        filter,
        appsink.upcast(),
    ])
}
