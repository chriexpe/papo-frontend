//! Videos in messages as they come and go (`docs/design.md` §14.5, `docs/papo-desktop.md` §5):
//! which are read (fetched and probed for their length, size and first frame), which play and
//! how (muted autoplay of clips of a minute or less while at least half shows; with sound once
//! pressed), one sound at a time, what pauses when it leaves the view, and which keep a live
//! pipeline, at most [`MAX_LIVE`].
//!
//! Pure: it decides; `shell::videos` fetches, probes, opens and drops players as it says and
//! reports back. §5's states map onto it: visible (shown at least half), near-visible (wanted:
//! read and its poster kept), offscreen (wanted no more), suspended (live but paused), released
//! (no pipeline; its place remembered).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::media_store::MediaKey;
use crate::playback::{MediaInfo, PlaybackFailure};

mod view;
pub(crate) use view::{VideoShown, VideoView};

/// Live pipelines (playing or paused) the conversation keeps at once.
pub(crate) const MAX_LIVE: usize = 3;

/// The longest clip that plays by itself (§14.5: a minute).
pub(crate) const AUTOPLAY_MAX_MS: u64 = 60_000;

/// The share of a video that must show for it to play by itself (§14.5: at least half).
pub(crate) const SHOWN_SHARE: f32 = 0.5;

/// Probes running at once. The SDK admits two media fetches per profile: one stays for
/// pictures.
const MAX_PROBING: usize = 1;

/// Read videos remembered once no view wants them (their length and size, not their frames).
const REMEMBERED: usize = 256;

/// What a view drawing a video asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VideoWant {
    pub(crate) key: MediaKey,
    /// The box in device pixels: the first frame is read to fit it.
    pub(crate) box_px: [u32; 2],
    /// An album's cell: never plays there (§14.4).
    pub(crate) in_album: bool,
    /// Sensitive and hidden: never plays until revealed.
    pub(crate) hidden: bool,
}

/// How far a video's reading has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Reading {
    /// Waiting for a probe slot.
    Queued,
    /// Its bytes are fetched and probed.
    Probing,
    /// Read: its first frame is on its surface.
    Read(MediaInfo),
    /// It can't be read or played.
    Failed(PlaybackFailure),
}

/// What the person (or autoplay) asked of a video.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of the video's independent conditions"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Intent {
    /// Pressed (play, sound, a seek): autoplay no longer decides for it.
    by_person: bool,
    playing: bool,
    muted: bool,
    /// Reached its end with sound: shows its last frame until played again.
    ended: bool,
}

/// One video.
struct Entry {
    reading: Reading,
    want: VideoWant,
    intent: Intent,
    /// Where it was when its pipeline went, in milliseconds.
    position_ms: u64,
    /// The share of it shown, 0 to 1.
    shown: f32,
    /// The last tick a view wanted it.
    used: u64,
}

impl Entry {
    fn duration_ms(&self) -> Option<u64> {
        match self.reading {
            Reading::Read(info) => info.duration_ms,
            _ => None,
        }
    }

    /// Whether it plays by itself: a read clip of a minute or less, alone (not in an album),
    /// not hidden, at least half shown, autoplay on, nobody having pressed it.
    fn autoplays(&self, autoplay: bool) -> bool {
        self.may_autoplay(autoplay) && self.shown >= SHOWN_SHARE
    }

    /// Whether it would play by itself were at least half of it shown.
    fn may_autoplay(&self, autoplay: bool) -> bool {
        autoplay
            && !self.intent.by_person
            && !self.want.in_album
            && !self.want.hidden
            && self
                .duration_ms()
                .is_some_and(|duration_ms| duration_ms <= AUTOPLAY_MAX_MS)
    }
}

/// How a live video should be now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Desired {
    pub(crate) playing: bool,
    pub(crate) muted: bool,
    /// Loops: a muted autoplay (§14.5).
    pub(crate) looping: bool,
    /// Where it starts if its pipeline must be opened.
    pub(crate) start_ms: u64,
}

/// A probe to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProbeJob {
    pub(crate) key: MediaKey,
    pub(crate) box_px: [u32; 2],
}

/// Videos' state.
#[derive(Default)]
pub(crate) struct VideoStore {
    entries: HashMap<MediaKey, Entry>,
    queue: VecDeque<MediaKey>,
    probing: HashSet<MediaKey>,
    /// Autoplay videos (the setting, reduced motion applied).
    autoplay: bool,
    /// The viewer shows this one: it pauses inline meanwhile (§14.8).
    in_viewer: Option<MediaKey>,
    /// The window is off screen (minimised, hidden, covered whole): nothing plays or stays
    /// live (§5's suspended state for the whole conversation).
    window_hidden: bool,
    tick: u64,
}

impl VideoStore {
    /// Whether short videos play by themselves.
    pub(crate) fn set_autoplay(&mut self, autoplay: bool) {
        self.autoplay = autoplay;
    }

    /// The window went off screen (`false`) or came back. Leaving it is leaving the view: what
    /// played with sound pauses and stays paused (§14.5); autoplays wait to show again.
    pub(crate) fn set_window_shown(&mut self, shown: bool) {
        self.window_hidden = !shown;
        if !shown {
            for entry in self.entries.values_mut() {
                leave_view(&mut entry.intent);
            }
        }
    }

    /// The shown views draw `wants` now (visible and near rows); others may go. Returns the
    /// probes to start.
    pub(crate) fn want(&mut self, wants: impl IntoIterator<Item = VideoWant>) -> Vec<ProbeJob> {
        self.tick += 1;
        let tick = self.tick;
        let mut wanted = HashSet::new();
        for want in wants {
            wanted.insert(want.key);
            let entry = self.entries.entry(want.key).or_insert_with(|| Entry {
                reading: Reading::Queued,
                want,
                intent: Intent::default(),
                position_ms: 0,
                shown: 0.0,
                used: tick,
            });
            entry.want = want;
            entry.used = tick;
            // New, or its frames were let go: read (again) in turn.
            if entry.reading == Reading::Queued && !self.queue.contains(&want.key) {
                self.queue.push_back(want.key);
            }
        }
        // A video no longer wanted shows no more: what it played with sound stays paused.
        for (key, entry) in &mut self.entries {
            if !wanted.contains(key) {
                entry.shown = 0.0;
                leave_view(&mut entry.intent);
            }
        }
        self.queue.retain(|key| wanted.contains(key));
        self.forget(&wanted);
        self.next_probes()
    }

    /// Each wanted video's shown share (0 to 1), from its box between the conversation's pills.
    pub(crate) fn shown(&mut self, shares: &HashMap<MediaKey, f32>) {
        for (key, entry) in &mut self.entries {
            let shown = shares.get(key).copied().unwrap_or_default();
            // Leaving the view: under half after at least half, or gone entirely.
            let left = (entry.shown >= SHOWN_SHARE && shown < SHOWN_SHARE)
                || (entry.shown > 0.0 && shown <= 0.0);
            entry.shown = shown;
            if left {
                leave_view(&mut entry.intent);
            }
        }
    }

    /// Probe `key` ended: read as `outcome`. Returns the probes to start next.
    pub(crate) fn probed(
        &mut self,
        key: MediaKey,
        outcome: Result<MediaInfo, PlaybackFailure>,
    ) -> Vec<ProbeJob> {
        self.probing.remove(&key);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.reading = match outcome {
                Ok(info) => Reading::Read(info),
                Err(failure) => Reading::Failed(failure),
            };
        }
        self.next_probes()
    }

    /// A probe or a fetch was dropped before it ran (no SDK): it is asked for again later.
    pub(crate) fn probe_dropped(&mut self, key: MediaKey) {
        self.probing.remove(&key);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.reading = Reading::Queued;
        }
    }

    /// Video `key`'s frames were let go (its first frame with them): it is read again when a
    /// view wants it next.
    pub(crate) fn frames_dropped(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key)
            && matches!(entry.reading, Reading::Read(_))
        {
            entry.reading = Reading::Queued;
        }
    }

    /// Retry on a failed video: read it again. Returns the probes to start.
    pub(crate) fn retry(&mut self, key: MediaKey) -> Vec<ProbeJob> {
        if let Some(entry) = self.entries.get_mut(&key)
            && matches!(entry.reading, Reading::Failed(_))
        {
            entry.reading = Reading::Queued;
            entry.intent = Intent::default();
            self.queue.push_back(key);
        }
        self.next_probes()
    }

    /// The person pressed play (or Space, K): it plays, from the start once it ended. A video
    /// that waited (a long one, autoplay off) plays with sound, muting every other (§14.5); one
    /// already muted stays muted.
    pub(crate) fn play(&mut self, key: MediaKey) {
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        if !matches!(entry.reading, Reading::Read(_)) {
            return;
        }
        if entry.intent.ended {
            entry.position_ms = 0;
        }
        // Play keeps an autoplay's (or the person's) silence; a video that waited gets sound.
        let muted = if entry.intent.by_person {
            entry.intent.muted
        } else {
            entry.autoplays(self.autoplay)
        };
        entry.intent = Intent {
            by_person: true,
            playing: true,
            muted,
            ended: false,
        };
        if !muted {
            self.only_sound(key);
        }
    }

    /// The person paused it.
    pub(crate) fn pause(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key) {
            // A paused autoplay stays muted.
            entry.intent.muted = !entry.intent.by_person || entry.intent.muted;
            entry.intent.by_person = true;
            entry.intent.playing = false;
        }
    }

    /// The person turned its sound on (it plays, every other one is muted) or off.
    pub(crate) fn set_sound(&mut self, key: MediaKey, on: bool) {
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        if !matches!(entry.reading, Reading::Read(_)) {
            return;
        }
        let was_playing = entry.intent.playing || entry.autoplays(self.autoplay);
        entry.intent.by_person = true;
        entry.intent.muted = !on;
        if on {
            entry.intent.playing = true;
            entry.intent.ended = false;
            self.only_sound(key);
        } else {
            entry.intent.playing = was_playing;
        }
    }

    /// The person moved it to `position_ms` (the seek bar): it stays as it was, playing or not.
    pub(crate) fn seeked(&mut self, key: MediaKey, position_ms: u64) {
        if let Some(entry) = self.entries.get_mut(&key) {
            // An autoplay keeps playing muted: a seek is not a request for sound.
            let playing = entry.intent.playing || entry.autoplays(self.autoplay);
            let muted = !entry.intent.by_person || entry.intent.muted;
            entry.intent = Intent {
                by_person: true,
                playing,
                muted,
                ended: false,
            };
            entry.position_ms = position_ms;
        }
    }

    /// Its player reached its end (it doesn't loop: it played with sound).
    pub(crate) fn ended(&mut self, key: MediaKey) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.intent.playing = false;
            entry.intent.ended = true;
            entry.position_ms = entry.duration_ms().unwrap_or(entry.position_ms);
        }
    }

    /// Its player failed while playing.
    pub(crate) fn player_failed(&mut self, key: MediaKey, failure: PlaybackFailure) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.reading = Reading::Failed(failure);
            entry.intent = Intent::default();
        }
    }

    /// Its pipeline went at `position_ms`: it starts there when it opens again.
    pub(crate) fn released(&mut self, key: MediaKey, position_ms: u64) {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.position_ms = position_ms;
        }
    }

    /// Another sound starts (an audio, the viewer): every video with sound is muted.
    pub(crate) fn mute_all(&mut self) {
        for entry in self.entries.values_mut() {
            if entry.intent.by_person && !entry.intent.muted {
                entry.intent.muted = true;
            }
        }
    }

    /// The viewer shows `key` (or nothing): it pauses inline meanwhile; when the viewer lets it
    /// go at `position_ms` it waits there, paused if it was pressed, else as autoplay says.
    pub(crate) fn set_in_viewer(&mut self, key: Option<MediaKey>, position_ms: Option<u64>) {
        if let Some(left) = self.in_viewer.filter(|left| Some(*left) != key)
            && let Some(entry) = self.entries.get_mut(&left)
        {
            if let Some(position_ms) = position_ms {
                entry.position_ms = position_ms;
            }
            entry.intent.playing = false;
            entry.intent.ended = false;
        }
        self.in_viewer = key;
    }

    /// The live pipelines the conversation should have, and how each should be, at most
    /// [`MAX_LIVE`]: what plays (what the person played first, then what shows most), then
    /// what the person paused while it still shows (suspended: it resumes at once). The rest
    /// are released; their last frame stays drawn.
    pub(crate) fn desired(&self) -> HashMap<MediaKey, Desired> {
        if self.window_hidden {
            return HashMap::new();
        }
        let mut live: Vec<(&MediaKey, &Entry, bool)> = self
            .entries
            .iter()
            .filter(|(key, _)| Some(**key) != self.in_viewer)
            .filter_map(|(key, entry)| {
                let playing = self.plays(entry);
                // Paused but kept while it shows: what you paused, and an autoplay under the
                // half line (scrolling across it must not tear its pipeline down and up).
                let waiting = (entry.intent.by_person && !entry.intent.ended)
                    || entry.may_autoplay(self.autoplay);
                let suspended = !playing
                    && waiting
                    && entry.shown > 0.0
                    && matches!(entry.reading, Reading::Read(_));
                (playing || suspended).then_some((key, entry, playing))
            })
            .collect();
        live.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then(b.1.intent.by_person.cmp(&a.1.intent.by_person))
                .then(b.1.shown.total_cmp(&a.1.shown))
                .then(a.0.attachment.cmp(&b.0.attachment))
        });
        live.truncate(MAX_LIVE);
        live.into_iter()
            .map(|(key, entry, playing)| {
                let autoplaying = !entry.intent.by_person;
                (
                    *key,
                    Desired {
                        playing,
                        muted: autoplaying || entry.intent.muted,
                        looping: autoplaying,
                        start_ms: entry.position_ms,
                    },
                )
            })
            .collect()
    }

    /// Whether `entry` plays now: as autoplay says, or as the person asked (muted only while
    /// it shows; with sound until it leaves the view, which pauses it).
    fn plays(&self, entry: &Entry) -> bool {
        if !matches!(entry.reading, Reading::Read(_)) {
            return false;
        }
        if !entry.intent.by_person {
            return entry.autoplays(self.autoplay);
        }
        entry.intent.playing && (!entry.intent.muted || entry.shown > 0.0)
    }

    /// What views read.
    pub(crate) fn view(&self) -> VideoView {
        VideoView::of(self.entries.iter().map(|(key, entry)| {
            let playing = Some(*key) != self.in_viewer && self.plays(entry);
            (
                *key,
                VideoShown {
                    reading: entry.reading,
                    playing,
                    muted: !entry.intent.by_person || entry.intent.muted,
                    pressed: entry.intent.by_person,
                    ended: entry.intent.ended,
                    position_ms: entry.position_ms,
                },
            )
        }))
    }

    /// The video whose sound plays: only `key` keeps it.
    fn only_sound(&mut self, key: MediaKey) {
        for (other, entry) in &mut self.entries {
            if *other != key && entry.intent.by_person && !entry.intent.muted {
                entry.intent.muted = true;
            }
        }
    }

    fn next_probes(&mut self) -> Vec<ProbeJob> {
        let mut jobs = Vec::new();
        while self.probing.len() < MAX_PROBING {
            let Some(key) = self.queue.pop_front() else {
                break;
            };
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            if entry.reading != Reading::Queued {
                continue;
            }
            entry.reading = Reading::Probing;
            self.probing.insert(key);
            jobs.push(ProbeJob {
                key,
                box_px: entry.want.box_px,
            });
        }
        jobs
    }

    /// Drops the least recently wanted videos past [`REMEMBERED`], never a wanted one or one
    /// being probed.
    fn forget(&mut self, wanted: &HashSet<MediaKey>) {
        let extra = self.entries.len().saturating_sub(REMEMBERED);
        if extra == 0 {
            return;
        }
        let mut idle: Vec<(u64, MediaKey)> = self
            .entries
            .iter()
            .filter(|(key, _)| !wanted.contains(key) && !self.probing.contains(key))
            .map(|(key, entry)| (entry.used, *key))
            .collect();
        idle.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in idle.into_iter().take(extra) {
            self.entries.remove(&key);
        }
    }
}

/// A video leaves the view: one with sound pauses and stays paused; a muted one pauses until
/// it comes back (autoplay's own rule does that for those nobody pressed).
fn leave_view(intent: &mut Intent) {
    if intent.by_person && intent.playing && !intent.muted {
        intent.playing = false;
    }
}

#[cfg(test)]
mod tests;
