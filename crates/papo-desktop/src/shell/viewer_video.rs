//! A video in the viewer (`docs/design.md` §14.8): it continues from where the inline one was,
//! with sound at the remembered volume, and the inline one pauses meanwhile (one sound at a
//! time: every other video is muted); its 48 pt control bar shows while paused and fades after
//! 2 s of a still pointer while it plays; Space or K, J and L, M. Owns the viewer's player; the
//! inline videos are `shell::videos`', the viewer's pictures `shell::viewer`'s.

use std::sync::Arc;

use papo_sdk::MediaSource;
use rune::{NativeSurface, Tx};

use super::Shell;
use crate::design::{VIEWER_BAR_IDLE_S, VIEWER_SEEK_STEP_MS};
use crate::media_store::MediaKey;
use crate::message_media::video::VideoPress;
use crate::playback::{PlaybackFailure, Player, PlayerSetup, PlayerState, PlayerStatus, SeekMode};
use crate::video_store::Reading;
use crate::video_surface::SurfaceSink;

/// How often the viewer's time is drawn anew while its video plays and its bar shows, in
/// seconds.
const VIEWER_TICK_S: f64 = 0.25;

/// The viewer's video.
pub(crate) struct ViewerVideo {
    pub(crate) key: MediaKey,
    pub(crate) surface: NativeSurface,
    player: Option<Box<dyn Player>>,
    serial: u64,
    /// Its state and time as last read.
    pub(crate) status: PlayerStatus,
    pub(crate) muted: bool,
    /// While it plays, when its bar goes (on the window's clock); none once gone.
    bar_until_s: Option<f64>,
    /// Its volume slider shows.
    pub(crate) volume_open: bool,
}

impl ViewerVideo {
    /// Whether the control bar shows: always while paused, else until the pointer rests.
    pub(crate) fn bar_shown(&self) -> bool {
        self.status.state != PlayerState::Playing || self.bar_until_s.is_some()
    }

    /// Why it can't play, if it can't.
    pub(crate) fn failure(&self) -> Option<PlaybackFailure> {
        match self.status.state {
            PlayerState::Failed(failure) => Some(failure),
            _ => None,
        }
    }
}

impl Shell {
    /// The viewer stepped, opened or closed: its video follows the item it shows.
    pub(super) fn follow_viewer_video(&mut self, tx: &mut Tx) {
        let item = self
            .viewer
            .as_ref()
            .and_then(crate::viewer::Viewer::item)
            .filter(|item| item.video)
            .map(|item| item.key);
        if self.viewer_video.as_ref().map(|video| video.key) == item {
            return;
        }
        self.stop_viewer_video();
        if let Some(key) = item {
            self.start_viewer_video(key, tx);
        }
    }

    /// Lets the viewer's video go: the inline one waits where it stopped.
    pub(super) fn stop_viewer_video(&mut self) {
        let Some(video) = self.viewer_video.take() else {
            return;
        };
        let position_ms = video
            .player
            .as_ref()
            .map_or(video.status.position_ms, |player| {
                player.status().position_ms
            });
        self.videos.store.set_in_viewer(None, Some(position_ms));
        self.reconcile_videos();
    }

    /// Starts video `key` in the viewer, where the inline one was, with sound.
    fn start_viewer_video(&mut self, key: MediaKey, tx: &mut Tx) {
        let shown = self.videos.view().get(key).copied();
        let start_ms = shown.map_or(0, |shown| shown.position_ms);
        let failure = match shown.map(|shown| shown.reading) {
            Some(Reading::Failed(failure)) => Some(failure),
            _ => None,
        };
        self.videos.store.set_in_viewer(Some(key), None);
        self.videos.store.mute_all();
        self.reconcile_videos();
        self.serial_viewer += 1;
        let serial = self.serial_viewer;
        let surface = NativeSurface::new();
        if let Some(waker) = self.videos.waker() {
            surface.set_waker(waker);
        }
        let state = failure.map_or(PlayerState::Loading, PlayerState::Failed);
        self.viewer_video = Some(ViewerVideo {
            key,
            surface: surface.clone(),
            player: None,
            serial,
            status: PlayerStatus {
                state,
                position_ms: start_ms,
                duration_ms: shown.and_then(|shown| shown.duration_ms()),
            },
            muted: false,
            bar_until_s: Some(tx.clock() + VIEWER_BAR_IDLE_S),
            volume_open: false,
        });
        if failure.is_some() {
            return;
        }
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let engine = self.videos.engine();
        let volume = self.videos.volume.get();
        let room = self.viewer_room;
        let scale = self.scale;
        let frame_px = room.map(|side| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the room's side in device pixels"
            )]
            let px = (side * scale).round().max(2.0) as u32;
            px
        });
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let source = MediaSource::Attachment(key.attachment);
            let bytes = match client.session(key.account) {
                Ok(session) => {
                    let media = session.media();
                    match media.cached(source.clone()).await {
                        Ok(data) => Some(data.value.bytes),
                        Err(_) => media
                            .fetch(source, false)
                            .await
                            .ok()
                            .map(|data| data.value.bytes),
                    }
                }
                Err(_) => None,
            };
            let changed_updates = updates.clone();
            let changed: crate::playback::Changed = Arc::new(move || {
                changed_updates.send(move |shell| shell.viewer_video_changed(serial));
            });
            let player = bytes.map(|bytes| {
                engine.open(
                    bytes,
                    PlayerSetup {
                        frame_px: Some(frame_px),
                        playing: true,
                        start_ms,
                        muted: false,
                        volume,
                        looping: true,
                    },
                    Arc::new(SurfaceSink::new(surface)),
                    changed,
                )
            });
            updates.send(move |shell| {
                let Some(video) = shell
                    .viewer_video
                    .as_mut()
                    .filter(|video| video.serial == serial)
                else {
                    // Stepped or closed meanwhile: dropping the player stops it.
                    return false;
                };
                match player {
                    Some(player) => video.player = Some(player),
                    None => video.status.state = PlayerState::Failed(PlaybackFailure::Broken),
                }
                true
            });
        });
    }

    /// The viewer's player (`serial`) changed. Returns whether the view changes.
    fn viewer_video_changed(&mut self, serial: u64) -> bool {
        let Some(video) = self
            .viewer_video
            .as_mut()
            .filter(|video| video.serial == serial)
        else {
            return false;
        };
        if let Some(player) = &video.player {
            video.status = player.status();
        }
        true
    }

    /// A press on the viewer's video controls (or its keys).
    pub(crate) fn viewer_video_press(&mut self, press: VideoPress, tx: &mut Tx) {
        let Some(video) = self.viewer_video.as_mut() else {
            return;
        };
        let clock = tx.clock();
        video.bar_until_s = Some(clock + VIEWER_BAR_IDLE_S);
        let Some(player) = &video.player else {
            return;
        };
        match press {
            VideoPress::Play => {
                player.play();
                video.status.state = PlayerState::Playing;
            }
            VideoPress::Pause => {
                player.pause();
                video.status.state = PlayerState::Paused;
            }
            VideoPress::Sound(on) => {
                video.muted = !on;
                player.set_muted(!on);
            }
            VideoPress::Seek { share, done } => {
                let duration_ms = video.status.duration_ms.unwrap_or_default();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "a share of a video's length"
                )]
                let position_ms = (f64::from(share.clamp(0.0, 1.0)) * duration_ms as f64) as u64;
                let mode = if done {
                    SeekMode::Accurate
                } else {
                    SeekMode::Keyframe
                };
                player.seek(position_ms, mode);
                video.status.position_ms = position_ms;
            }
            VideoPress::SeekBy(by_ms) => {
                let end = video.status.duration_ms.unwrap_or(u64::MAX);
                let position_ms = video
                    .status
                    .position_ms
                    .saturating_add_signed(by_ms)
                    .min(end);
                player.seek(position_ms, SeekMode::Keyframe);
                video.status.position_ms = position_ms;
            }
            VideoPress::Volume(volume) => {
                player.set_volume(volume);
                if video.muted && volume > 0.0 {
                    video.muted = false;
                    player.set_muted(false);
                }
                if let Some(write) = self.videos.volume.set(volume) {
                    self.write_video_volume(write, tx);
                }
            }
            VideoPress::Viewer | VideoPress::Retry => {}
        }
    }

    /// Space or K, J and L, M on the viewer's video (§14.8). Returns whether it took the key.
    pub(super) fn viewer_video_key(&mut self, typed: &str, tx: &mut Tx) -> bool {
        let Some(video) = &self.viewer_video else {
            return false;
        };
        let step = i64::try_from(VIEWER_SEEK_STEP_MS).unwrap_or(i64::MAX);
        let press = match typed {
            " " | "k" | "K" if video.status.state == PlayerState::Playing => VideoPress::Pause,
            " " | "k" | "K" => VideoPress::Play,
            "j" | "J" => VideoPress::SeekBy(-step),
            "l" | "L" => VideoPress::SeekBy(step),
            "m" | "M" => VideoPress::Sound(video.muted),
            _ => return false,
        };
        self.viewer_video_press(press, tx);
        true
    }

    /// The pointer moved over the viewer: its video's bar shows again for 2 s. Returns whether
    /// the view changes.
    pub(crate) fn viewer_pointer_moved(&mut self, tx: &mut Tx) -> bool {
        let Some(video) = self.viewer_video.as_mut() else {
            return false;
        };
        let was = video.bar_shown();
        video.bar_until_s = Some(tx.clock() + VIEWER_BAR_IDLE_S);
        tx.wake_after(std::time::Duration::from_secs_f64(VIEWER_BAR_IDLE_S));
        !was
    }

    /// The pointer or keyboard came to (or left) the viewer's volume.
    pub(crate) fn viewer_volume_open(&mut self, on: bool) -> bool {
        let Some(video) = self.viewer_video.as_mut() else {
            return false;
        };
        let changed = video.volume_open != on;
        video.volume_open = on;
        changed
    }

    /// The window's clock: the viewer's video time, and its bar gone after a still pointer.
    /// Returns whether the view changes.
    pub(super) fn poll_viewer_video(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        let Some(video) = self.viewer_video.as_mut() else {
            return false;
        };
        let before = (video.status, video.bar_shown());
        if let Some(player) = &video.player {
            video.status = player.status();
        }
        if video.bar_until_s.is_some_and(|until| until <= clock_s) {
            video.bar_until_s = None;
        }
        if video.status.state == PlayerState::Playing {
            let wait_s = video.bar_until_s.map_or(1.0 - clock_s.fract(), |until| {
                (until - clock_s).min(VIEWER_TICK_S)
            });
            tx.wake_after(std::time::Duration::from_secs_f64(wait_s.max(0.05)));
        }
        (video.status, video.bar_shown()) != before
    }

    /// The window went off screen or came back: video in the conversation follows the store's
    /// rule, and the viewer's pauses while hidden (it stays paused when the window returns).
    pub(super) fn video_window_shown(&mut self, shown: bool, tx: &mut Tx) {
        self.videos.store.set_window_shown(shown);
        self.reconcile_videos();
        let playing = self
            .viewer_video
            .as_ref()
            .is_some_and(|video| video.status.state == PlayerState::Playing);
        if !shown && playing {
            self.viewer_video_press(VideoPress::Pause, tx);
        }
    }
}
