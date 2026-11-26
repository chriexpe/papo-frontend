//! Audio's presses and keys (`docs/design.md` §14.5, §12): the player's Play and Pause, the
//! waveform's seek (a press, ←/→, or the pointer's hairline over it), Space or K on a focused
//! message with audio, and Retry from the media menu for audio that couldn't be read. One sound
//! at a time: playing audio mutes any video's sound.

use papo_sdk::Attachment;
use papo_sdk_ui::AttachmentKind;
use rune::Tx;

use super::Shell;
use crate::audio_store::{AudioKey, Reading};
use crate::media_store::MediaKey;
use crate::message_media::audio::AudioPress;
use crate::message_media::received::{MediaAt, attachment_id};
use crate::playback::PlaybackFailure;
use crate::timeline::TimelineKey;

impl Shell {
    /// A press on audio `key`'s player (at `at`).
    pub(super) fn press_audio(
        &mut self,
        at: MediaAt,
        key: AudioKey,
        press: AudioPress,
        tx: &mut Tx,
    ) {
        // A row may still name an unsent audio its queue has since sent.
        let key = self.audio.store.resolve(key);
        match press {
            AudioPress::Play => {
                if self.audio.store.play(key) {
                    self.audio.conversation = Some(TimelineKey {
                        account: at.account,
                        channel: at.channel,
                    });
                    // One sound at a time: a video's sound goes quiet (it keeps playing), and
                    // so do provider players.
                    self.videos.store.mute_all();
                    self.reconcile_videos();
                    self.quiet_players();
                }
            }
            AudioPress::Pause => self.audio.store.pause(key),
            AudioPress::Seek(share) => {
                let duration_ms = self.audio.store.duration_ms(key).unwrap_or_default();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "a share of an audio's length"
                )]
                let position_ms = (f64::from(share.clamp(0.0, 1.0)) * duration_ms as f64) as u64;
                self.seek_audio(key, position_ms);
            }
            AudioPress::SeekBy(by_ms) => {
                let position_ms = self
                    .audio
                    .store
                    .position_ms(key)
                    .saturating_add_signed(by_ms);
                self.seek_audio(key, position_ms);
            }
            AudioPress::Hover(share) => {
                let hover = share.map(|share| (key, share.clamp(0.0, 1.0)));
                let leaving_another =
                    share.is_none() && self.audio.hover.is_some_and(|(hovered, _)| hovered != key);
                if self.audio.hover == hover || leaving_another {
                    tx.live_only();
                } else {
                    self.audio.hover = hover;
                }
                return;
            }
        }
        self.reconcile_audio();
    }

    /// Moves audio `key` to `position_ms` (within its length): its player now, else where it
    /// opens.
    fn seek_audio(&mut self, key: AudioKey, position_ms: u64) {
        let position_ms = self.audio.store.seek(key, position_ms);
        self.seek_audio_player(key, position_ms);
    }

    /// Space or K on focused message `id` of timeline `key` (yours unsent too) plays or pauses
    /// its first audio (a video in the message takes the keys first, `message_video_key`).
    /// Returns whether it took the key.
    pub(super) fn message_audio_key(
        &mut self,
        key: TimelineKey,
        id: papo_sdk::Uuid,
        typed: &str,
        tx: &mut Tx,
    ) -> bool {
        if !matches!(typed, " " | "k" | "K") {
            return false;
        }
        let at = MediaAt {
            surface: crate::message_media::received::MediaSurface::Timeline,
            account: key.account,
            channel: key.channel,
            message: id,
            index: 0,
        };
        let Some((index, audio)) = self
            .received_audio(at)
            .or_else(|| self.unsent_audio(key, id))
        else {
            return false;
        };
        let playing = self
            .audio
            .store
            .shown(audio)
            .is_some_and(|shown| shown.playing);
        let press = if playing {
            AudioPress::Pause
        } else {
            AudioPress::Play
        };
        self.press_audio(MediaAt { index, ..at }, audio, press, tx);
        true
    }

    /// Received message `at`'s first audio: its place and key.
    fn received_audio(&self, at: MediaAt) -> Option<(usize, AudioKey)> {
        let message = self.message_at(at)?;
        let attachments = message.attachments.as_deref().unwrap_or_default();
        attachments
            .iter()
            .enumerate()
            .find_map(|(index, attachment)| {
                (AttachmentKind::of(attachment) == AttachmentKind::Audio)
                    .then(|| attachment_key(at, attachment).map(|key| (index, key)))
                    .flatten()
            })
    }

    /// Whether attachment `at` is audio that couldn't be read for a reason another read may
    /// get past (not its format): the media menu offers Retry.
    pub(super) fn audio_retries(&self, at: MediaAt, attachment: &Attachment) -> bool {
        let Some(key) = attachment_key(at, attachment) else {
            return false;
        };
        self.audio.store.shown(key).is_some_and(|shown| {
            matches!(
                shown.reading,
                Reading::Failed(PlaybackFailure::Broken | PlaybackFailure::Unavailable)
            )
        })
    }

    /// The media menu's Retry on audio `at`: it is read again.
    pub(super) fn retry_audio(&mut self, at: MediaAt, attachment: &Attachment) {
        if let Some(key) = attachment_key(at, attachment) {
            self.audio.store.retry(key);
            self.pump_media_reads();
        }
    }
}

/// Received attachment `at`'s audio key, when it has an ID.
fn attachment_key(at: MediaAt, attachment: &Attachment) -> Option<AudioKey> {
    Some(AudioKey::Received(MediaKey::attachment(
        at.account,
        attachment_id(attachment)?,
    )))
}
