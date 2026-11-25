//! Video and audio playback for messages (`docs/design.md` §14.5, `docs/papo-desktop.md` §5):
//! one interface, shaped after Rune's `Player` (play, pause, seek, looping, muted, state, time),
//! with one implementation, GStreamer's (`gst_engine`). Owns opening encoded bytes, probing them
//! (length, size, first frame) and running a player; frames leave as `papo-call`'s
//! [`PackedNv12`] through a [`FrameSink`]. Nothing here names a GStreamer or Rune type: drawing
//! the frames is `video_surface`'s, and which media play, wait or are let go is the shell's.
//!
//! Every call returns at once: GStreamer starts, builds and tears pipelines down on threads of
//! its own, so the UI thread never waits on the registry, a decoder or the audio device.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(crate) use papo_call::PackedNv12;
use papo_sdk_ui::Waveform;

mod frames;
mod gst_engine;
mod memory;
mod peaks;
mod pipeline;
mod probe;
mod source;
#[cfg(test)]
mod tests;

/// Why media can't play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PlaybackFailure {
    /// No decoder (or demuxer) for its format here: it can be saved and played elsewhere.
    Unsupported,
    /// Its bytes are damaged, or a decoder gave up.
    Broken,
    /// GStreamer itself couldn't start, or the audio device refused.
    Unavailable,
}

/// What a probe found in media.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct MediaInfo {
    /// How long it plays, when the container says.
    pub(crate) duration_ms: Option<u64>,
    /// The video's size in pixels as displayed (pixel aspect applied), when it has video.
    pub(crate) video_px: Option<[u32; 2]>,
    /// Whether it has sound.
    pub(crate) has_audio: bool,
}

/// A probe's answer: what the media is, and its first frame when it has video.
pub(crate) struct Probe {
    pub(crate) info: MediaInfo,
    /// The first frame, scaled to fit the size asked for.
    pub(crate) poster: Option<PackedNv12>,
}

/// What reading audio found: its length, and its waveform when it was decoded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AudioRead {
    /// How long it plays: as the container says, or as decoded.
    pub(crate) duration_ms: Option<u64>,
    /// Its waveform, finished; none when it is longer than the read was allowed to decode, or
    /// the read was stopped.
    pub(crate) waveform: Option<Waveform>,
}

/// How exact a seek is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeekMode {
    /// To the exact time asked (decodes from the keyframe before it): a click on the seek bar.
    Accurate,
    /// To the nearest keyframe: quick, for steps such as J and L.
    Keyframe,
}

/// Where a player is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerState {
    /// Opening: no frame yet.
    Loading,
    /// Stopped where it is, its frame shown.
    Paused,
    /// Playing.
    Playing,
    /// Reached its end without looping.
    Ended,
    /// Can't play.
    Failed(PlaybackFailure),
}

/// A player's state and time, read together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerStatus {
    pub(crate) state: PlayerState,
    /// Where it is, in milliseconds from the start.
    pub(crate) position_ms: u64,
    /// How long it plays, once known.
    pub(crate) duration_ms: Option<u64>,
}

/// Where a player's frames go. Called on a streaming thread, newest frame last; a slow sink
/// only ever falls behind by the one frame the pipeline holds.
pub(crate) trait FrameSink: Send + Sync + 'static {
    /// The next frame to show.
    fn frame(&self, frame: PackedNv12);
}

/// Called (on a player's own thread) when a player's state, length or failure changed, so the
/// interface reads its status again. Time passing is not a change.
pub(crate) type Changed = Arc<dyn Fn() + Send + Sync>;

/// How a player starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlayerSetup {
    /// The box frames are fitted into, in device pixels: their shape kept, never enlarged.
    /// `None` delivers them at the stream's own size.
    pub(crate) frame_px: Option<[u32; 2]>,
    /// Starts playing as soon as it can (else paused on its first frame).
    pub(crate) playing: bool,
    /// Where it starts, in milliseconds.
    pub(crate) start_ms: u64,
    pub(crate) muted: bool,
    /// 0 to 1.
    pub(crate) volume: f32,
    /// Starts again at its end.
    pub(crate) looping: bool,
}

/// A running player. Dropping it stops it and lets its pipeline go, off the UI thread.
pub(crate) trait Player: Send {
    /// Plays from where it is (from the start once it ended).
    fn play(&self);
    /// Pauses where it is.
    fn pause(&self);
    /// Moves to `position_ms`, playing or paused as it was.
    fn seek(&self, position_ms: u64, mode: SeekMode);
    fn set_looping(&self, looping: bool);
    fn set_muted(&self, muted: bool);
    /// 0 to 1; muting is apart, so the volume comes back when sound is turned on again.
    fn set_volume(&self, volume: f32);
    /// Frames fitted into `box_px` (device pixels) from now on: a larger box (the viewer, a
    /// larger Video size) sharpens them, a smaller one makes them cheaper.
    fn set_frame_box(&self, box_px: [u32; 2]);
    /// Its state and time now.
    fn status(&self) -> PlayerStatus;
}

/// What plays media: GStreamer in the app, a fake in tests.
pub(crate) trait Engine: Send + Sync + 'static {
    /// Reads `bytes` (blocking: call it off the UI thread): length, size, sound, and the first
    /// frame scaled to fit `poster_px`.
    ///
    /// # Errors
    /// The media can't play here, and why.
    fn probe(&self, bytes: Arc<[u8]>, poster_px: [u32; 2]) -> Result<Probe, PlaybackFailure>;

    /// Starts a player of `bytes` as `setup` says, its frames to `sink`; without one it plays
    /// the sound only.
    fn open(
        &self,
        bytes: Arc<[u8]>,
        setup: PlayerSetup,
        sink: Option<Arc<dyn FrameSink>>,
        changed: Changed,
    ) -> Box<dyn Player>;

    /// Reads audio `bytes` (blocking: call it off the UI thread): its length, then, unless it
    /// is longer than `ahead_max_ms`, its waveform, decoded as fast as it goes. `progress` gets
    /// the waveform so far now and then (at most four times a second). Setting `stop` ends the
    /// read early: it then answers what it has, the waveform left out.
    ///
    /// # Errors
    /// The media has no sound, or can't be read here, and why.
    fn read_audio(
        &self,
        bytes: Arc<[u8]>,
        ahead_max_ms: Option<u64>,
        progress: &(dyn Fn(Waveform) + Send + Sync),
        stop: &AtomicBool,
    ) -> Result<AudioRead, PlaybackFailure>;
}

/// The app's engine: GStreamer, started on first use.
pub(crate) fn engine() -> Arc<dyn Engine> {
    Arc::new(gst_engine::GstEngine)
}

/// The frame size for media of `source_px` drawn at most `box_px` (device pixels): its shape
/// kept, never enlarged, each side even (NV12 halves them) and at least two.
pub(crate) fn fitted_frame(source_px: [u32; 2], box_px: [u32; 2]) -> [u32; 2] {
    let [width, height] = source_px.map(|side| f64::from(side.max(1)));
    let [box_width, box_height] = box_px.map(|side| f64::from(side.max(2)));
    let scale = (box_width / width).min(box_height / height).min(1.0);
    [width, height].map(|side| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive side at most a u32's, rounded"
        )]
        let side = (side * scale).round() as u32;
        (side / 2 * 2).max(2)
    })
}
