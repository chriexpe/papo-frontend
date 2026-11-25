//! Your unsent message's audio (`docs/design.md` §14.3, §14.5): it plays like a sent one, read
//! from its staged file, once staged. When the message is sent its audio goes on under the
//! server's attachment (a playing one keeps playing, its waveform goes to the SDK's cache);
//! when it is discarded it stops. Reading and playing are `audio_reads`' and `audio`'s.
//!
//! The account's queue and the timeline report a send separately, in either order: whichever
//! comes first hands the audio over, and the store still finds it by its unsent key for the
//! other.

use papo_sdk::{AccountId, Operation, OperationState, OutgoingSnapshot, Uuid};

use super::Shell;
use super::audio_reads::cache_key;
use crate::audio_store::AudioKey;
use crate::composer::tray::TileKind;
use crate::timeline::TimelineKey;

impl Shell {
    /// Account `account`'s queue changed: each audio of a message now sent goes on under the
    /// server's attachment, and the audio of messages that left the queue unsent (`gone`,
    /// discarded) stops. The timeline may show the send first: `want_media` hands over too.
    pub(super) fn unsent_audio_follow(
        &mut self,
        account: AccountId,
        outgoing: &OutgoingSnapshot,
        gone: &[Uuid],
    ) {
        let sent: Vec<(AudioKey, AudioKey)> = outgoing
            .operations
            .iter()
            .flat_map(|operation| self.sent_audio(account, operation))
            .collect();
        for (from, to) in sent {
            self.audio_sent(from, to);
        }
        // A sent message's audio moved on above, so what is left of a gone one was discarded.
        for local in gone {
            self.audio_discarded(*local);
        }
    }

    /// The audio of `account`'s message `operation`, once sent: each unsent file and the
    /// server's attachment it is now, while something of it is known here.
    pub(super) fn sent_audio(
        &self,
        account: AccountId,
        operation: &Operation,
    ) -> Vec<(AudioKey, AudioKey)> {
        if operation.state != OperationState::Sent {
            return Vec::new();
        }
        self.unsent_audio_files(operation.id)
            .into_iter()
            .map(|index| {
                let from = AudioKey::Unsent {
                    account,
                    local: operation.id,
                    index,
                };
                (from, AudioKey::of_operation(account, operation, index))
            })
            .filter(|(from, to)| from != to && self.audio.store.knows(*from))
            .collect()
    }

    /// Unsent audio `from` was sent as `to`: what it knows, its player, its waveform decode and
    /// the pointer over it go on under `to`. A read still running from the local file (let go
    /// once sent) is read again from the server, and a player still opening from it reopens
    /// there; a read of the server's copy is let go when the local one already read it.
    pub(super) fn audio_sent(&mut self, from: AudioKey, to: AudioKey) {
        let from_read = self.audio.store.is_read(from);
        if let Some(waveform) = self.audio.store.finished_waveform(from)
            && let Some(cache) = cache_key(to)
        {
            self.audio.waveforms.insert(cache, waveform);
        }
        if let Some(reading) = &self.audio.reading
            && (reading.key == from || (reading.key == to && from_read))
        {
            reading.stop();
        }
        self.audio.store.sent(from, to);
        let audio = &mut self.audio;
        let keys = [
            audio.live.as_mut().map(|live| &mut live.key),
            audio.filling.as_mut().map(|filling| &mut filling.key),
            audio.hover.as_mut().map(|(key, _)| key),
        ];
        for key in keys.into_iter().flatten() {
            if *key == from {
                *key = to;
            }
        }
        self.reconcile_audio();
    }

    /// Unsent message `local` was discarded: its audio stops, its read and decode with it.
    fn audio_discarded(&mut self, local: Uuid) {
        self.audio.store.discarded(local);
        for running in [&self.audio.reading, &self.audio.filling]
            .into_iter()
            .flatten()
        {
            if running.key.is_unsent_of(local) {
                running.stop();
            }
        }
        if self
            .audio
            .hover
            .is_some_and(|(key, _)| key.is_unsent_of(local))
        {
            self.audio.hover = None;
        }
        self.reconcile_audio();
    }

    /// The audio of `account`'s unsent message `operation` that can be read now: what the row
    /// draws as audio, already sent, or with its staged file in the account's queue (an
    /// unstaged one still copies).
    pub(super) fn unsent_audio_wanted(
        &self,
        account: AccountId,
        operation: &Operation,
    ) -> Vec<AudioKey> {
        self.unsent_audio_files(operation.id)
            .into_iter()
            .map(|index| AudioKey::of_operation(account, operation, index))
            .filter(|key| matches!(key, AudioKey::Received(_)) || self.audio_source(*key).is_some())
            .collect()
    }

    /// Whether account `account`'s queue now holds unsent audio that the shown conversation
    /// draws but isn't read yet: `want_media` then asks for it (the timeline may have drawn it
    /// before the queue staged it).
    pub(super) fn unsent_audio_waits(&self, account: AccountId) -> bool {
        let Some(outgoing) = self
            .activity
            .get(&account)
            .and_then(|activity| activity.outgoing.as_deref())
        else {
            return false;
        };
        self.tabs
            .shown()
            .is_some_and(|open| open.key.account == account)
            && outgoing.operations.iter().any(|operation| {
                self.unsent_audio_files(operation.id)
                    .into_iter()
                    .any(|index| {
                        let key = AudioKey::of_operation(account, operation, index);
                        !self.audio.store.knows(self.audio.store.resolve(key))
                    })
            })
    }

    /// Your unsent message `local` (its echo) in timeline `key`: its first audio's place and
    /// key.
    pub(super) fn unsent_audio(&self, key: TimelineKey, local: Uuid) -> Option<(usize, AudioKey)> {
        let open = self.tabs.shown().filter(|open| open.key == key)?;
        let operation = open.echoes.get(local)?.operation();
        let index = *self.unsent_audio_files(operation.id).first()?;
        Some((index, AudioKey::of_operation(key.account, operation, index)))
    }

    /// Which of unsent message `local`'s files are drawn as audio.
    fn unsent_audio_files(&self, local: Uuid) -> Vec<usize> {
        self.unsent.get(local).map_or_else(Vec::new, |entry| {
            entry
                .files
                .iter()
                .enumerate()
                .filter(|(_, file)| file.kind == TileKind::Audio)
                .map(|(index, _)| index)
                .collect()
        })
    }
}
