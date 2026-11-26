//! The shell's side of audio in messages (`docs/design.md` §14.5): what is near the view, the
//! one player (sound only, from the SDK's cache, or an unsent audio's staged file), its clock,
//! and the rules that cross into video: one sound at a time, the window off screen (Keep
//! playing when hidden), leaving the conversation. The rules themselves are `audio_store`'s;
//! reading and waveforms are `audio_reads`', presses and keys `audio_presses`', an unsent
//! audio's end `unsent_audio`'s.

use std::sync::Arc;

use papo_sdk::{AccountId, Attachment};
use papo_sdk_ui::{AUDIO_FILE_BARS, AttachmentKind, DEFAULT_CACHE_ENTRIES, WaveformCache};
use rune::Tx;

use super::Shell;
use super::audio_reads::Running;
use crate::audio_store::{AudioDesired, AudioKey, AudioStore};
use crate::media_store::MediaKey;
use crate::message_media::received::attachment_id;
use crate::playback::{PlaybackFailure, Player, PlayerSetup, PlayerState, SeekMode};
use crate::timeline::TimelineKey;

/// The live player.
pub(super) struct Live {
    /// Its audio; moves to the server's attachment when the audio is sent.
    pub(super) key: AudioKey,
    player: Box<dyn Player>,
    /// Numbers its opening, so a late answer from an earlier one is let go.
    serial: u64,
    /// It was last told to play.
    playing: bool,
}

/// Audio in messages.
pub(crate) struct Audio {
    pub(super) store: AudioStore,
    /// Finished waveforms, in the SDK's cache: an audio read again needs only its length.
    pub(super) waveforms: WaveformCache,
    pub(super) live: Option<Live>,
    /// The player opening, by its serial.
    pub(super) opening: Option<(AudioKey, u64)>,
    /// Numbers players and waveform decodes.
    pub(super) serial: u64,
    /// The read holding `media_reads`' slot.
    pub(super) reading: Option<Running>,
    /// Long audio's waveform decoding while it plays.
    pub(super) filling: Option<Running>,
    /// The conversation the audio with the player is in: leaving it pauses the audio.
    pub(super) conversation: Option<TimelineKey>,
    /// The waveform the pointer is over, and where along it (0 to 1).
    pub(crate) hover: Option<(AudioKey, f32)>,
    /// What the playing audio's row shows: its second and its played bars.
    shown: Option<(u64, usize)>,
    /// When the playing audio's time is read next, on the window's clock (other wakes, a
    /// video's frames among them, don't read it).
    next_clock_s: Option<f64>,
}

impl Default for Audio {
    fn default() -> Audio {
        Audio {
            store: AudioStore::default(),
            waveforms: WaveformCache::new(DEFAULT_CACHE_ENTRIES),
            live: None,
            opening: None,
            serial: 0,
            reading: None,
            filling: None,
            conversation: None,
            hover: None,
            shown: None,
            next_clock_s: None,
        }
    }
}

impl Shell {
    /// The audio the shown timeline draws (its visible and near rows, and the open pins) is
    /// `keys`, in order: what isn't read is read in turn. Showing another conversation leaves
    /// the one the audio plays in.
    pub(super) fn want_audio(&mut self, keys: Vec<AudioKey>) {
        let shown = self.tabs.shown().map(|open| open.key);
        if self.audio.conversation.is_some() && self.audio.conversation != shown {
            self.leave_audio();
        }
        if let Some(reading) = &self.audio.reading
            && !keys.contains(&reading.key)
        {
            reading.stop();
        }
        self.audio.store.want(keys);
        self.pump_media_reads();
    }

    /// Opens, steers and lets go of the one player as the store wants it now.
    pub(super) fn reconcile_audio(&mut self) {
        let desired = self.audio.store.desired();
        let wanted = desired.map(|want| want.key);
        if self
            .audio
            .live
            .as_ref()
            .is_some_and(|live| Some(live.key) != wanted)
            && let Some(live) = self.audio.live.take()
        {
            self.audio
                .store
                .released(live.key, live.player.status().position_ms);
            self.audio.shown = None;
        }
        if self
            .audio
            .opening
            .is_some_and(|(key, _)| Some(key) != wanted)
        {
            self.audio.opening = None;
        }
        let Some(want) = desired else {
            return;
        };
        match self.audio.live.as_mut() {
            Some(live) => {
                if want.playing != live.playing {
                    if want.playing {
                        live.player.play();
                    } else {
                        live.player.pause();
                    }
                }
                live.playing = want.playing;
                // Playing or paused, its clock is read at the next poll.
                self.audio.next_clock_s = None;
            }
            None if self.audio.opening.is_none() => self.open_audio(want),
            None => {}
        }
    }

    /// Opens the player for `want`, from the audio's original in the SDK's cache (an unsent
    /// one's staged file); long audio without its waveform decodes it meanwhile.
    fn open_audio(&mut self, want: AudioDesired) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let key = want.key;
        let Some(source) = self.audio_source(key) else {
            self.audio
                .store
                .player_failed(key, PlaybackFailure::Unavailable);
            return;
        };
        self.audio.serial += 1;
        let serial = self.audio.serial;
        self.audio.opening = Some((key, serial));
        let fill = self.start_fill(key);
        let engine = self.videos.engine();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let bytes = source.bytes(&client, true).await;
            let changed_updates = updates.clone();
            let changed: crate::playback::Changed = Arc::new(move || {
                changed_updates.send(move |shell| shell.audio_changed(serial));
            });
            let setup = PlayerSetup {
                frame_px: None,
                playing: want.playing,
                start_ms: want.start_ms,
                muted: false,
                volume: 1.0,
                looping: false,
            };
            let player = bytes
                .clone()
                .map(|bytes| engine.open(bytes, setup, None, changed));
            if let (Some(bytes), Some(fill)) = (bytes, fill) {
                super::audio_reads::fill_waveform(&engine, bytes, fill, &updates);
            }
            updates.send(move |shell| {
                shell.audio_opened(serial, want, player);
                true
            });
        });
    }

    /// Player `serial` opened (or its bytes couldn't be read), as `applied` asked. Its audio
    /// is the one opening now, which may have been sent meanwhile.
    pub(super) fn audio_opened(
        &mut self,
        serial: u64,
        applied: AudioDesired,
        player: Option<Box<dyn Player>>,
    ) {
        let Some((key, _)) = self
            .audio
            .opening
            .take_if(|(_, opening)| *opening == serial)
        else {
            // No longer wanted, or superseded: dropping it stops it.
            return;
        };
        let Some(player) = player else {
            self.audio.store.player_failed(key, PlaybackFailure::Broken);
            return;
        };
        // A seek made while it opened.
        let position_ms = self.audio.store.position_ms(key);
        if position_ms != applied.start_ms {
            player.seek(position_ms, SeekMode::Accurate);
        }
        self.audio.live = Some(Live {
            key,
            player,
            serial,
            playing: applied.playing,
        });
        // It may have ended or failed before this answer landed.
        self.audio_changed(serial);
        self.reconcile_audio();
    }

    /// The player (`serial`) changed: it ended or failed. Returns whether views change.
    pub(super) fn audio_changed(&mut self, serial: u64) -> bool {
        let Some(live) = self
            .audio
            .live
            .as_ref()
            .filter(|live| live.serial == serial)
        else {
            return false;
        };
        let key = live.key;
        let state = live.player.status().state;
        if matches!(state, PlayerState::Ended | PlayerState::Failed(_)) {
            // Let go first: its place at the end must not be kept as where it resumes.
            self.audio.live = None;
            self.audio.shown = None;
        }
        match state {
            PlayerState::Ended => self.audio.store.ended(key),
            PlayerState::Failed(failure) => self.audio.store.player_failed(key, failure),
            PlayerState::Loading | PlayerState::Paused | PlayerState::Playing => return true,
        }
        self.reconcile_audio();
        true
    }

    /// Moves the live player of audio `key` to `position_ms`, if it has one.
    pub(super) fn seek_audio_player(&mut self, key: AudioKey, position_ms: u64) {
        if let Some(live) = self.audio.live.as_mut().filter(|live| live.key == key) {
            live.player.seek(position_ms, SeekMode::Accurate);
            self.audio.shown = None;
            self.audio.next_clock_s = None;
        }
    }

    /// One sound at a time: a video's sound, or the viewer's, pauses audio.
    pub(super) fn pause_audio(&mut self) {
        if self.audio.store.is_playing() {
            self.audio.store.pause_all();
            self.reconcile_audio();
        }
    }

    /// The conversation the audio plays in was left: it pauses where it is, its player goes.
    fn leave_audio(&mut self) {
        if let Some(live) = &self.audio.live {
            self.audio
                .store
                .released(live.key, live.player.status().position_ms);
        }
        self.audio.store.leave();
        self.audio.conversation = None;
        self.reconcile_audio();
    }

    /// The window went off screen or came back, or Keep playing when hidden changed: what
    /// plays follows the setting (muted autoplays always stop while hidden).
    pub(super) fn media_window_rules(&mut self) {
        let keep = self.conversations.settings().keep_playing;
        self.videos
            .store
            .set_window_shown(self.window_shown, keep.video());
        self.reconcile_videos();
        if !self.window_shown && !keep.audio() {
            self.pause_audio();
        }
    }

    /// The window's clock: the playing audio's time and played bars, read as they change.
    /// Returns whether views change, and asks for the next tick while audio plays.
    pub(super) fn poll_audio(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        // Another conversation shows (a channel, a server, or none): the audio pauses.
        let shown = self.tabs.shown().map(|open| open.key);
        if self.audio.conversation.is_some() && self.audio.conversation != shown {
            self.leave_audio();
            return true;
        }
        let Some(live) = &self.audio.live else {
            self.audio.next_clock_s = None;
            return false;
        };
        if self.audio.next_clock_s.is_some_and(|next| clock_s < next) {
            return false;
        }
        let key = live.key;
        let status = live.player.status();
        if status.state != PlayerState::Playing {
            self.audio.next_clock_s = None;
            return false;
        }
        self.audio.store.moved(key, status.position_ms);
        let Some(shown) = self.audio.store.shown(key) else {
            return false;
        };
        let now = (
            status.position_ms / 1000,
            shown.played_bars(AUDIO_FILE_BARS),
        );
        // The next second, or the next bar, whichever comes first.
        let to_second_ms = 1000 - status.position_ms % 1000;
        // Every bar played (an estimated length passed) or no length: seconds only.
        let to_bar_ms = shown
            .duration_ms()
            .filter(|duration_ms| *duration_ms > 0 && now.1 < AUDIO_FILE_BARS)
            .map_or(u64::MAX, |duration_ms| {
                let bars = u64::try_from(AUDIO_FILE_BARS).unwrap_or(64);
                let next = (u64::try_from(now.1).unwrap_or(0) + 1) * duration_ms;
                next.div_ceil(bars).saturating_sub(status.position_ms)
            });
        let wait = std::time::Duration::from_millis(to_second_ms.min(to_bar_ms).max(50));
        self.audio.next_clock_s = Some(clock_s + wait.as_secs_f64());
        tx.wake_after(wait);
        let changed = self.audio.shown != Some(now);
        self.audio.shown = Some(now);
        changed
    }
}

/// The audio among `attachments` of account `account`: its keys, in order.
pub(super) fn audio_wanted(account: AccountId, attachments: &[Attachment]) -> Vec<AudioKey> {
    attachments
        .iter()
        .filter(|attachment| AttachmentKind::of(attachment) == AttachmentKind::Audio)
        .filter_map(attachment_id)
        .map(|attachment| AudioKey::Received(MediaKey::attachment(account, attachment)))
        .collect()
}
