//! An audio player's waveform (`docs/design.md` §14.5): the loudness of each stretch of the
//! audio, reduced from decoded samples, and a bounded cache of finished waveforms keyed by
//! account and attachment, so every client draws the same bars from the same audio.
//! Owns only the reduction and the cache; decoding, playback and drawing are
//! the client's. The SDK never decodes.
//!
//! The loudness of a stretch is its RMS over every channel. Samples are summed into a fixed
//! number of windows ([`WINDOWS`]); when more audio arrives than the windows were sized for,
//! neighbouring windows merge, which keeps RMS exact. So memory stays the same for any length,
//! and a waveform can be read while it is still being decoded.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use papo_sdk::{AccountId, Uuid};

/// The windows a waveform keeps, whatever the audio's length. A bar's edges fall inside a
/// window, so finer windows keep a short transient from leaking into the next bar: at 1,024,
/// twenty bars over six seconds land within 0.01 of an exact reduction.
pub const WINDOWS: usize = 1024;

/// The bars an audio file's player draws (§14.5).
pub const AUDIO_FILE_BARS: usize = 64;

/// The longest audio whose waveform is decoded before it plays, in milliseconds (§14.5: ten
/// minutes). Longer audio stays flat until it is first played and fills in as it decodes.
pub const MAX_AHEAD_DURATION_MS: u64 = 10 * 60 * 1000;

/// The finished waveforms a [`WaveformCache`] keeps by default: 128 of 16 KiB each.
pub const DEFAULT_CACHE_ENTRIES: usize = 128;

/// One window's samples: the sum of their squares and how many frames it holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Window {
    squares: f64,
    frames: u64,
}

impl Window {
    fn merge(self, other: Self) -> Self {
        Self {
            squares: self.squares + other.squares,
            frames: self.frames + other.frames,
        }
    }
}

/// Reduces decoded audio to a [`Waveform`] as it arrives.
///
/// Give it the audio's length in frames when known (a frame is one sample per channel), so a
/// partial read spreads its bars over the whole audio and shows where the decoded part ends.
/// Windows grow as audio arrives whatever was expected: a length from container metadata may
/// be wrong, and windows sized for it would leave the bars too coarse.
#[derive(Clone, Debug)]
pub struct WaveformBuilder {
    windows: Vec<Window>,
    /// Frames each window takes, at least one; doubles when the windows fill.
    window_frames: u64,
    /// Frames pushed so far.
    frames: u64,
    /// The audio's length in frames, when the client knows it.
    expected_frames: Option<u64>,
}

impl WaveformBuilder {
    /// A builder for audio of `expected_frames` frames, or of unknown length.
    #[must_use]
    pub fn new(expected_frames: Option<u64>) -> Self {
        Self {
            windows: vec![Window::default(); WINDOWS],
            window_frames: 1,
            frames: 0,
            expected_frames: expected_frames.filter(|frames| *frames > 0),
        }
    }

    /// Adds interleaved samples of `channels` channels, each nominally in −1…1. A frame's
    /// loudness is the mean square of its channels. Samples beyond ±1 count as ±1, samples
    /// that are not finite as silence, and a trailing partial frame is ignored; no channels
    /// adds nothing.
    pub fn push(&mut self, samples: &[f32], channels: u16) {
        if channels == 0 {
            return;
        }
        for frame in samples.chunks_exact(usize::from(channels)) {
            let squares: f64 = frame
                .iter()
                .map(|sample| {
                    let sample = if sample.is_finite() {
                        f64::from(sample.clamp(-1.0, 1.0))
                    } else {
                        0.0
                    };
                    sample * sample
                })
                .sum();
            self.add_frame(squares / f64::from(channels));
        }
    }

    fn add_frame(&mut self, mean_square: f64) {
        if self.frames / self.window_frames >= WINDOWS as u64 {
            // Halving makes room: the frames so far now fill half the windows.
            self.halve();
        }
        let index = usize::try_from(self.frames / self.window_frames).unwrap_or(WINDOWS - 1);
        if let Some(window) = self.windows.get_mut(index) {
            window.squares += mean_square;
            window.frames += 1;
            self.frames += 1;
        }
    }

    /// The windows are full: each pair merges into one and windows take twice the frames.
    fn halve(&mut self) {
        for index in 0..WINDOWS / 2 {
            self.windows[index] = self.windows[2 * index].merge(self.windows[2 * index + 1]);
        }
        self.windows[WINDOWS / 2..].fill(Window::default());
        self.window_frames = self.window_frames.saturating_mul(2);
    }

    /// The waveform so far, for audio still decoding: bars past the decoded part are unknown.
    #[must_use]
    pub fn snapshot(&self) -> Waveform {
        self.waveform(false)
    }

    /// The finished waveform: the decoded audio is the whole audio, whatever length was
    /// expected.
    #[must_use]
    pub fn finish(self) -> Waveform {
        self.waveform(true)
    }

    fn waveform(&self, complete: bool) -> Waveform {
        let used = usize::try_from(self.frames.div_ceil(self.window_frames))
            .unwrap_or(WINDOWS)
            .min(WINDOWS);
        let total_frames = if complete {
            self.frames
        } else {
            self.expected_frames.unwrap_or(self.frames).max(self.frames)
        };
        Waveform {
            windows: Arc::from(&self.windows[..used]),
            window_frames: self.window_frames,
            decoded_frames: self.frames,
            total_frames,
            complete,
        }
    }
}

/// An audio's loudness over its length, from a [`WaveformBuilder`]: cheap to clone, read as
/// bars at any count.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    /// The used windows, in order; each covers `window_frames` frames (the last may hold
    /// fewer).
    windows: Arc<[Window]>,
    window_frames: u64,
    decoded_frames: u64,
    /// The length the bars spread over: the expected length while decoding, the decoded one
    /// once finished.
    total_frames: u64,
    /// Made by [`WaveformBuilder::finish`]: the decoder reached the audio's end.
    complete: bool,
}

impl Waveform {
    /// Whether the audio was decoded to its end ([`WaveformBuilder::finish`]); a snapshot never
    /// is, even when it has decoded as much as was expected.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// `count` bars spread evenly over the audio's length, each the RMS of its stretch over
    /// the loudest known bar, in 0…1. A bar whose stretch hasn't been decoded yet is `None`
    /// (drawn flat, §14.5); every bar is `None` before any audio, and every known bar is 0 for
    /// silence. Known bars rescale as louder audio arrives.
    #[must_use]
    pub fn bars(&self, count: usize) -> Vec<Option<f32>> {
        if count == 0 {
            return Vec::new();
        }
        if self.total_frames == 0 || self.decoded_frames == 0 {
            return vec![None; count];
        }
        let total = u128::from(self.total_frames);
        let levels: Vec<Option<f64>> = (0..count as u128)
            .map(|bar| {
                let start = total * bar / count as u128;
                let end = (total * (bar + 1) / count as u128).max(start + 1);
                self.level(start, end)
            })
            .collect();
        let loudest = levels.iter().flatten().copied().fold(0.0_f64, f64::max);
        levels
            .into_iter()
            .map(|level| {
                level.map(|level| {
                    if loudest > 0.0 {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "a share in 0…1 fits an f32"
                        )]
                        let share = (level / loudest).clamp(0.0, 1.0) as f32;
                        share
                    } else {
                        0.0
                    }
                })
            })
            .collect()
    }

    /// The RMS of frames `start..end`, or `None` when the stretch starts past the decoded
    /// audio. A window the stretch only partly covers counts by the share it covers, as if its
    /// loudness were even across it: the waveform's resolution is its windows.
    #[expect(
        clippy::cast_precision_loss,
        reason = "frame counts stay far below 2^52 (a century of 48 kHz audio)"
    )]
    fn level(&self, start: u128, end: u128) -> Option<f64> {
        if start >= u128::from(self.decoded_frames) {
            return None;
        }
        let window_frames = u128::from(self.window_frames);
        let first = usize::try_from(start / window_frames).ok()?;
        let mut squares = 0.0;
        let mut frames = 0.0;
        for (index, window) in self.windows.iter().enumerate().skip(first) {
            let window_start = index as u128 * window_frames;
            if window_start >= end || window.frames == 0 {
                break;
            }
            let window_end = window_start + u128::from(window.frames);
            let covered = window_end.min(end).saturating_sub(window_start.max(start));
            let share = covered as f64 / window.frames as f64;
            squares += window.squares * share;
            frames += window.frames as f64 * share;
        }
        Some(if frames > 0.0 {
            (squares / frames).sqrt()
        } else {
            0.0
        })
    }
}

/// Where a waveform belongs: the account that can read the attachment, and the attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaveformKey {
    /// The account the audio was read through.
    pub account: AccountId,
    /// The audio attachment.
    pub attachment: Uuid,
}

/// Finished waveforms, least recently read dropped first beyond its capacity. Clones share one
/// cache, so the thread that decodes can store what the interface reads. Only brief map
/// operations hold its lock.
#[derive(Clone, Debug)]
pub struct WaveformCache {
    state: Arc<Mutex<CacheState>>,
}

#[derive(Debug)]
struct CacheState {
    capacity: usize,
    entries: HashMap<WaveformKey, Waveform>,
    /// Keys from least to most recently used; each key appears once.
    order: VecDeque<WaveformKey>,
}

impl Default for WaveformCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_ENTRIES)
    }
}

impl WaveformCache {
    /// A cache keeping at most `capacity` waveforms (at least one).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(CacheState {
                capacity: capacity.max(1),
                entries: HashMap::new(),
                order: VecDeque::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The finished waveform kept for `key`, marking it recently used.
    #[must_use]
    pub fn get(&self, key: WaveformKey) -> Option<Waveform> {
        let mut state = self.lock();
        let waveform = state.entries.get(&key)?.clone();
        state.touch(key);
        Some(waveform)
    }

    /// Keeps `waveform` for `key`, replacing an older one. Only a complete waveform is kept: a
    /// partial one is the decoder's to hold, so it never outlives the decode that makes it.
    pub fn insert(&self, key: WaveformKey, waveform: Waveform) {
        if !waveform.is_complete() {
            return;
        }
        let mut state = self.lock();
        state.entries.insert(key, waveform);
        state.touch(key);
        while state.entries.len() > state.capacity {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.entries.remove(&oldest);
        }
    }

    /// Drops every waveform read through `account`, for when the account is removed.
    pub fn forget_account(&self, account: AccountId) {
        let mut state = self.lock();
        state.entries.retain(|key, _| key.account != account);
        state.order.retain(|key| key.account != account);
    }

    /// How many waveforms are kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Whether nothing is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl CacheState {
    fn touch(&mut self, key: WaveformKey) {
        if let Some(position) = self.order.iter().position(|kept| *kept == key) {
            self.order.remove(position);
        }
        self.order.push_back(key);
    }
}

#[cfg(test)]
mod tests;
