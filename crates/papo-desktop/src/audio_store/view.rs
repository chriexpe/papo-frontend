//! How audio draws (`audio_store`'s view): each audio's reading, bars, time and play state,
//! read by the capsule (`message_media/audio`) and its row's memo.

use std::collections::HashMap;
use std::sync::Arc;

use super::AudioKey;
use super::Reading;

/// How one audio draws.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AudioShown {
    pub(crate) reading: Reading,
    /// Its bars, 0 to 1 each, `None` where not decoded yet; none before any is known (all flat).
    pub(crate) bars: Option<Arc<[Option<f32>]>>,
    /// Counts the changes to `bars`.
    pub(crate) bars_version: u64,
    pub(crate) position_ms: u64,
    pub(crate) playing: bool,
    /// Played since it last rested: its time shows what was played, not its length.
    pub(crate) played: bool,
}

impl AudioShown {
    /// An audio not read yet: flat bars, no time, at rest.
    pub(crate) fn unread() -> AudioShown {
        AudioShown {
            reading: Reading::Queued,
            bars: None,
            bars_version: 0,
            position_ms: 0,
            playing: false,
            played: false,
        }
    }

    /// Its length, once read.
    pub(crate) fn duration_ms(&self) -> Option<u64> {
        match self.reading {
            Reading::Read { duration_ms } => duration_ms,
            _ => None,
        }
    }

    /// The time it shows: its length at rest, the time played once played.
    pub(crate) fn shown_ms(&self) -> Option<u64> {
        if self.played {
            Some(self.position_ms)
        } else {
            self.duration_ms()
        }
    }

    /// How many of `count` bars were played.
    pub(crate) fn played_bars(&self, count: usize) -> usize {
        let Some(duration_ms) = self.duration_ms().filter(|duration_ms| *duration_ms > 0) else {
            return 0;
        };
        if !self.played {
            return 0;
        }
        let count_u64 = u64::try_from(count).unwrap_or(u64::MAX);
        let played = self.position_ms.min(duration_ms) * count_u64 / duration_ms;
        usize::try_from(played).unwrap_or(count)
    }
}

/// Every audio's look, cheap to clone.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AudioView {
    shown: Arc<HashMap<AudioKey, AudioShown>>,
    /// Unsent audio since sent, to the server's attachment (`AudioStore::sent`).
    moved: Arc<HashMap<AudioKey, AudioKey>>,
}

impl AudioView {
    pub(super) fn of(
        shown: impl Iterator<Item = (AudioKey, AudioShown)>,
        moved: HashMap<AudioKey, AudioKey>,
    ) -> AudioView {
        AudioView {
            shown: Arc::new(shown.collect()),
            moved: Arc::new(moved),
        }
    }

    /// How audio `key` draws: an unsent audio since sent draws as the server's attachment.
    pub(crate) fn get(&self, key: AudioKey) -> Option<&AudioShown> {
        self.shown.get(&self.resolve(key))
    }

    /// The audio `key` names now (`AudioStore::resolve`).
    pub(crate) fn resolve(&self, key: AudioKey) -> AudioKey {
        self.moved.get(&key).copied().unwrap_or(key)
    }
}
