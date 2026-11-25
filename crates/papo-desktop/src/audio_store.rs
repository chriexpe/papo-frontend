//! Audio in messages (`docs/design.md` §14.5): which audio is read, which one plays, and where
//! each is. Pure: reading and playing are the shell's (`shell/audio`), drawing
//! `message_media/audio`'s.
//!
//! - **Reading.** Audio near the view is read one at a time, in the order it shows: its length,
//!   and, when it plays for at most ten minutes ([`MAX_AHEAD_DURATION_MS`]), its waveform.
//!   Longer audio stays flat until it is first played, then fills in as it decodes.
//! - **Playing.** Only when pressed, one audio at a time. It keeps playing while its
//!   conversation shows, scrolled away included; leaving the conversation pauses it, as does a
//!   video's sound starting. One player at most: the audio playing, or the one paused last.
//! - **Time.** Each keeps its place. Once played it shows the time played; at its end it goes
//!   back to the start, at rest.
//! - **Failing.** Audio that can't be read or played is drawn as a file card (Christian,
//!   2026-11-25); Retry reads it again.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use papo_sdk_ui::{AUDIO_FILE_BARS, MAX_AHEAD_DURATION_MS, Waveform};

use crate::media_store::MediaKey;
use crate::playback::{AudioRead, PlaybackFailure};

/// Audio remembered (its length, waveform and place) beyond what is near the view.
const REMEMBERED: usize = 256;

/// Where reading an audio is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Reading {
    /// Waits for its turn.
    Queued,
    /// Being read.
    Running,
    /// Read: its length, when known.
    Read { duration_ms: Option<u64> },
    /// It can't be read or played here.
    Failed(PlaybackFailure),
}

/// A read to start: audio `key`, its waveform decoded unless it plays longer than
/// `ahead_max_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadJob {
    pub(crate) key: MediaKey,
    pub(crate) ahead_max_ms: Option<u64>,
}

/// How the one player should be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioDesired {
    pub(crate) key: MediaKey,
    pub(crate) playing: bool,
    /// Where a new player starts, in milliseconds.
    pub(crate) start_ms: u64,
}

struct Entry {
    reading: Reading,
    waveform: Option<Waveform>,
    /// Its bars as drawn, read from `waveform` when it changes.
    bars: Option<Arc<[Option<f32>]>>,
    /// Counts the changes to `bars`.
    bars_version: u64,
    position_ms: u64,
    /// Played since it last rested at the start: its time shows what was played.
    played: bool,
}

impl Entry {
    fn new() -> Entry {
        Entry {
            reading: Reading::Queued,
            waveform: None,
            bars: None,
            bars_version: 0,
            position_ms: 0,
            played: false,
        }
    }

    fn duration_ms(&self) -> Option<u64> {
        match self.reading {
            Reading::Read { duration_ms } => duration_ms,
            _ => None,
        }
    }

    fn set_waveform(&mut self, waveform: Waveform) {
        self.bars = Some(Arc::from(waveform.bars(AUDIO_FILE_BARS)));
        self.bars_version += 1;
        self.waveform = Some(waveform);
    }
}

/// Audio's rules.
#[derive(Default)]
pub(crate) struct AudioStore {
    entries: HashMap<MediaKey, Entry>,
    /// Audio near the view, in the order it shows.
    wanted: Vec<MediaKey>,
    /// Entries in the order they were last wanted, for forgetting the oldest.
    recent: VecDeque<MediaKey>,
    /// The audio with a player, and whether it plays.
    current: Option<(MediaKey, bool)>,
}

impl AudioStore {
    /// The shown views draw the audio `keys` now, in order (visible and near rows).
    pub(crate) fn want(&mut self, keys: impl IntoIterator<Item = MediaKey>) {
        self.wanted = keys.into_iter().collect();
        for key in &self.wanted {
            self.entries.entry(*key).or_insert_with(Entry::new);
            self.recent.retain(|recent| recent != key);
            self.recent.push_back(*key);
        }
        self.forget_old();
    }

    /// The next read to start, marked as running: the first wanted audio waiting.
    pub(crate) fn next_read(&mut self) -> Option<ReadJob> {
        let key = self.wanted.iter().copied().find(|key| {
            self.entries
                .get(key)
                .is_some_and(|entry| entry.reading == Reading::Queued)
        })?;
        let entry = self.entries.get_mut(&key)?;
        entry.reading = Reading::Running;
        Some(ReadJob {
            key,
            ahead_max_ms: Some(MAX_AHEAD_DURATION_MS),
        })
    }

    /// Audio `key`'s read ended.
    pub(crate) fn read(&mut self, key: MediaKey, outcome: Result<AudioRead, PlaybackFailure>) {
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        match outcome {
            Ok(read) => {
                entry.reading = Reading::Read {
                    duration_ms: read.duration_ms,
                };
                if let Some(waveform) = read.waveform {
                    entry.set_waveform(waveform);
                }
            }
            Err(failure) => {
                entry.reading = Reading::Failed(failure);
                if self.current.is_some_and(|(current, _)| current == key) {
                    self.current = None;
                }
            }
        }
    }

    /// Audio `key`'s read was let go before it ended (no longer wanted): it waits again.
    pub(crate) fn read_dropped(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key)
            && entry.reading == Reading::Running
        {
            entry.reading = Reading::Queued;
        }
    }

    /// Audio `key`'s waveform as decoded so far (it fills in while long audio plays), or
    /// finished. A finished one is never replaced by a partial one.
    pub(crate) fn filled(&mut self, key: MediaKey, waveform: Waveform) {
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        if entry.waveform.as_ref().is_some_and(Waveform::is_complete) {
            return;
        }
        entry.set_waveform(waveform);
    }

    /// Audio `key` was decoded to its end while it played: its waveform, and its length when
    /// the container didn't state one.
    pub(crate) fn fill_done(&mut self, key: MediaKey, read: AudioRead) {
        if let Some(entry) = self.entries.get_mut(&key)
            && entry.reading == (Reading::Read { duration_ms: None })
        {
            entry.reading = Reading::Read {
                duration_ms: read.duration_ms,
            };
        }
        if let Some(waveform) = read.waveform {
            self.filled(key, waveform);
        }
    }

    /// Whether audio `key` lacks its finished waveform: playing it decodes it.
    pub(crate) fn needs_waveform(&self, key: MediaKey) -> bool {
        self.entries
            .get(&key)
            .is_some_and(|entry| !entry.waveform.as_ref().is_some_and(Waveform::is_complete))
    }

    /// Plays audio `key` (from where it was) when it was read; it takes the one player. Returns
    /// whether it plays.
    pub(crate) fn play(&mut self, key: MediaKey) -> bool {
        let Some(entry) = self.entries.get_mut(&key) else {
            return false;
        };
        if !matches!(entry.reading, Reading::Read { .. }) {
            return false;
        }
        entry.played = true;
        self.current = Some((key, true));
        true
    }

    /// Pauses audio `key` where it is.
    pub(crate) fn pause(&mut self, key: MediaKey) {
        if let Some((current, playing)) = &mut self.current
            && *current == key
        {
            *playing = false;
        }
    }

    /// Pauses whatever plays: a video's sound started, or the window went off screen.
    pub(crate) fn pause_all(&mut self) {
        if let Some((_, playing)) = &mut self.current {
            *playing = false;
        }
    }

    /// Whether audio plays now.
    pub(crate) fn is_playing(&self) -> bool {
        self.current.is_some_and(|(_, playing)| playing)
    }

    /// Audio `key` moved to `position_ms` (a seek, clamped to its length), playing or paused as
    /// it was. Seeking a paused audio that never played shows its time from then on.
    pub(crate) fn seek(&mut self, key: MediaKey, position_ms: u64) -> u64 {
        let Some(entry) = self.entries.get_mut(&key) else {
            return position_ms;
        };
        let position_ms = entry
            .duration_ms()
            .map_or(position_ms, |duration_ms| position_ms.min(duration_ms));
        entry.position_ms = position_ms;
        entry.played = true;
        position_ms
    }

    /// Audio `key` is at `position_ms` while it plays (read from its player).
    pub(crate) fn moved(&mut self, key: MediaKey, position_ms: u64) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.position_ms = position_ms;
        }
    }

    /// Where audio `key` is.
    pub(crate) fn position_ms(&self, key: MediaKey) -> u64 {
        self.entries.get(&key).map_or(0, |entry| entry.position_ms)
    }

    /// Audio `key`'s length, when read.
    pub(crate) fn duration_ms(&self, key: MediaKey) -> Option<u64> {
        self.entries.get(&key).and_then(Entry::duration_ms)
    }

    /// Audio `key` reached its end: back at the start, at rest, its player let go.
    pub(crate) fn ended(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.position_ms = 0;
            entry.played = false;
        }
        if self.current.is_some_and(|(current, _)| current == key) {
            self.current = None;
        }
    }

    /// Audio `key`'s player failed: it is drawn as a file card from now on.
    pub(crate) fn player_failed(&mut self, key: MediaKey, failure: PlaybackFailure) {
        self.read(key, Err(failure));
    }

    /// Audio `key`'s player was let go at `position_ms`: it resumes there.
    pub(crate) fn released(&mut self, key: MediaKey, position_ms: u64) {
        self.moved(key, position_ms);
    }

    /// The conversation it plays in was left (another channel or server): it pauses, and its
    /// player goes.
    pub(crate) fn leave(&mut self) {
        self.current = None;
    }

    /// Audio `key` failed and the person asked again: it is read again.
    pub(crate) fn retry(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key)
            && matches!(entry.reading, Reading::Failed(_))
        {
            entry.reading = Reading::Queued;
        }
    }

    /// The one player as it should be: none, or the current audio playing or paused.
    pub(crate) fn desired(&self) -> Option<AudioDesired> {
        let (key, playing) = self.current?;
        let entry = self.entries.get(&key)?;
        Some(AudioDesired {
            key,
            playing,
            start_ms: entry.position_ms,
        })
    }

    /// What views read.
    pub(crate) fn view(&self) -> AudioView {
        AudioView::of(
            self.entries
                .iter()
                .map(|(key, entry)| (*key, self.shown_of(*key, entry))),
        )
    }

    /// How audio `key` draws now, when it is known: for a single audio, without building the
    /// whole view.
    pub(crate) fn shown(&self, key: MediaKey) -> Option<AudioShown> {
        self.entries
            .get(&key)
            .map(|entry| self.shown_of(key, entry))
    }

    fn shown_of(&self, key: MediaKey, entry: &Entry) -> AudioShown {
        AudioShown {
            reading: entry.reading,
            bars: entry.bars.clone(),
            bars_version: entry.bars_version,
            position_ms: entry.position_ms,
            playing: self.current == Some((key, true)),
            played: entry.played,
        }
    }

    /// Whether audio `key` is near the view now.
    pub(crate) fn is_wanted(&self, key: MediaKey) -> bool {
        self.wanted.contains(&key)
    }

    /// Forgets the oldest entries beyond [`REMEMBERED`], never one wanted or with the player.
    fn forget_old(&mut self) {
        while self.recent.len() > REMEMBERED {
            let Some(oldest) = self.recent.pop_front() else {
                return;
            };
            let kept = self.wanted.contains(&oldest)
                || self.current.is_some_and(|(current, _)| current == oldest);
            if kept {
                self.recent.push_back(oldest);
                return;
            }
            self.entries.remove(&oldest);
        }
    }
}

mod view;

pub(crate) use view::{AudioShown, AudioView};

#[cfg(test)]
mod tests;
