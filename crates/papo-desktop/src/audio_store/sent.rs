//! An unsent audio's end (`audio_store`): sent, what it knows (its length, waveform and place,
//! the player) goes on under the server's attachment, so it keeps playing, and the unsent key
//! keeps naming it there; discarded, it is forgotten and stops.

use papo_sdk::Uuid;

use super::{AudioKey, AudioStore, Reading};
use crate::playback::PlaybackFailure;

impl AudioStore {
    /// Unsent audio `from` was sent as `to`. Its read is kept when it got one; what only the
    /// local file said (a read still running, a file that couldn't be read) is read again from
    /// the server, and `to`'s own reading then stands when known, taking only `from`'s place.
    pub(crate) fn sent(&mut self, from: AudioKey, to: AudioKey) {
        let Some(mut entry) = self.entries.remove(&from) else {
            return;
        };
        self.moved.insert(from, to);
        let local_only = matches!(
            entry.reading,
            Reading::Running
                | Reading::Failed(PlaybackFailure::Broken | PlaybackFailure::Unavailable)
        );
        if local_only {
            entry.reading = Reading::Queued;
        }
        match self.entries.get_mut(&to) {
            Some(known) if !matches!(entry.reading, Reading::Read { .. }) => {
                if entry.played {
                    known.position_ms = entry.position_ms;
                    known.played = true;
                }
            }
            _ => {
                self.entries.insert(to, entry);
            }
        }
        let mut wanted = Vec::with_capacity(self.wanted.len());
        for key in self.wanted.drain(..) {
            let key = if key == from { to } else { key };
            if !wanted.contains(&key) {
                wanted.push(key);
            }
        }
        self.wanted = wanted;
        self.recent.retain(|key| *key != from && *key != to);
        self.recent.push_back(to);
        if let Some((current, _)) = &mut self.current
            && *current == from
        {
            *current = to;
        }
    }

    /// Unsent message `local` was discarded: its audio is forgotten, and stops if it plays.
    pub(crate) fn discarded(&mut self, local: Uuid) {
        self.entries.retain(|key, _| !key.is_unsent_of(local));
        self.wanted.retain(|key| !key.is_unsent_of(local));
        self.recent.retain(|key| !key.is_unsent_of(local));
        self.moved.retain(|from, _| !from.is_unsent_of(local));
        if self
            .current
            .is_some_and(|(current, _)| current.is_unsent_of(local))
        {
            self.current = None;
        }
    }
}
