//! The shell's side of videos in messages (`docs/design.md` §14.5, `docs/papo-desktop.md` §5):
//! which videos the shown timeline draws, reading each (its original through the SDK's cache,
//! probed off the UI thread, its first frame onto its surface), opening, steering and letting
//! go of players as `video_store` decides, what shows, presses on the controls, and the clock
//! the controls show. The rules are `video_store`'s; the frames `playback`'s.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, Attachment, MediaSource};
use papo_sdk_ui::{AttachmentKind, MediaBounds, media_box};
use rune::{NativeSurface, Tx};

use super::Shell;
use crate::media_store::MediaKey;
use crate::message_media::received::{MediaAt, attachment_id, is_hidden, split};
use crate::message_media::video::VideoPress;
use crate::playback::{Engine, Player, PlayerSetup, PlayerState, SeekMode};
use crate::video_store::{Desired, ProbeJob, VideoStore, VideoView, VideoWant};
use crate::video_surface::{SurfaceSink, byte_frame};
use crate::video_volume::VideoVolume;

/// Surfaces kept for videos no view wants (their first or last frame), most recent first.
const WARM_SURFACES: usize = 8;

/// How often a playing video's time is drawn anew while its controls show, in seconds and the
/// milliseconds it moves by; at rest it moves by whole seconds.
const CLOCK_TICK_S: f64 = 0.25;
const CLOCK_STEP_MS: u64 = 250;

/// A running player and what it was last told.
struct Live {
    player: Box<dyn Player>,
    applied: Desired,
    serial: u64,
    /// The box its frames are fitted into, in device pixels.
    box_px: Option<[u32; 2]>,
}

/// Videos' state on the shell.
pub(crate) struct Videos {
    pub(super) store: VideoStore,
    engine: Arc<dyn Engine>,
    surfaces: Arc<HashMap<MediaKey, NativeSurface>>,
    /// When each surface was last wanted, for which go first.
    surface_used: HashMap<MediaKey, u64>,
    players: HashMap<MediaKey, Live>,
    /// Players being opened, by the serial their answer carries.
    opening: HashMap<MediaKey, u64>,
    /// The box each player being opened fits its frames into.
    opening_box: HashMap<MediaKey, Option<[u32; 2]>>,
    serial: u64,
    rects: HashMap<MediaKey, [f32; 4]>,
    pub(super) attention: Option<MediaKey>,
    pub(super) volume_open: Option<MediaKey>,
    pub(super) volume: VideoVolume,
    /// Redraws the window when a surface gets a frame.
    waker: Option<rune::Waker>,
    /// Live players' positions, read on the window's clock.
    positions: HashMap<MediaKey, u64>,
    /// When the positions are read next, on the window's clock; none reads them at the next
    /// poll.
    pub(super) next_clock_s: Option<f64>,
    /// The system's reduced motion, as the view last saw it.
    reduced_motion: bool,
    tick: u64,
}

impl Default for Videos {
    fn default() -> Videos {
        Videos::with_engine(crate::playback::engine())
    }
}

impl Videos {
    /// Videos played by `engine` (GStreamer in the app, a fake in tests).
    pub(crate) fn with_engine(engine: Arc<dyn Engine>) -> Videos {
        Videos {
            store: VideoStore::default(),
            engine,
            surfaces: Arc::default(),
            surface_used: HashMap::new(),
            players: HashMap::new(),
            opening: HashMap::new(),
            opening_box: HashMap::new(),
            serial: 0,
            rects: HashMap::new(),
            attention: None,
            volume_open: None,
            volume: VideoVolume::default(),
            waker: None,
            positions: HashMap::new(),
            next_clock_s: None,
            reduced_motion: false,
            tick: 0,
        }
    }

    /// The window starts: frames landing on a surface redraw it.
    pub(crate) fn set_waker(&mut self, waker: rune::Waker) {
        self.waker = Some(waker);
    }

    /// The engine players are opened with.
    pub(super) fn engine(&self) -> Arc<dyn Engine> {
        Arc::clone(&self.engine)
    }

    /// What redraws the window when a surface gets a frame.
    pub(super) fn waker(&self) -> Option<rune::Waker> {
        self.waker.clone()
    }

    /// What views read.
    pub(crate) fn view(&self) -> VideoView {
        self.store.view().at_positions(&self.positions)
    }

    /// Each read video's frames.
    pub(crate) fn surfaces(&self) -> Arc<HashMap<MediaKey, NativeSurface>> {
        Arc::clone(&self.surfaces)
    }

    /// Video `key`'s surface, made the first time.
    fn surface(&mut self, key: MediaKey) -> NativeSurface {
        if let Some(surface) = self.surfaces.get(&key) {
            return surface.clone();
        }
        let surface = NativeSurface::new();
        if let Some(waker) = &self.waker {
            surface.set_waker(Arc::clone(waker));
        }
        Arc::make_mut(&mut self.surfaces).insert(key, surface.clone());
        surface
    }

    /// Lets go of the surfaces no view wants beyond the warm few, and tells the store their
    /// frames are gone.
    fn prune_surfaces(&mut self, wanted: &std::collections::HashSet<MediaKey>) {
        let tick = self.tick;
        for key in wanted {
            self.surface_used.insert(*key, tick);
        }
        let mut idle: Vec<(u64, MediaKey)> = self
            .surfaces
            .keys()
            .filter(|key| !wanted.contains(key) && !self.players.contains_key(key))
            .map(|key| {
                (
                    self.surface_used.get(key).copied().unwrap_or_default(),
                    *key,
                )
            })
            .collect();
        idle.sort_unstable_by_key(|(used, _)| std::cmp::Reverse(*used));
        for (_, key) in idle.into_iter().skip(WARM_SURFACES) {
            Arc::make_mut(&mut self.surfaces).remove(&key);
            self.surface_used.remove(&key);
            self.store.frames_dropped(key);
        }
    }
}

impl Shell {
    /// The videos the shown timeline draws (its visible and near rows, and the open pins) are
    /// `wants`: read what isn't, let go of what no longer shows.
    pub(super) fn want_videos(&mut self, wants: Vec<VideoWant>) {
        self.videos.tick += 1;
        let wanted: std::collections::HashSet<MediaKey> =
            wants.iter().map(|want| want.key).collect();
        self.videos.rects.retain(|key, _| wanted.contains(key));
        let autoplay = self
            .conversations
            .settings()
            .autoplays_videos(self.videos.reduced_motion);
        self.videos.store.set_autoplay(autoplay);
        // A larger Video size, window or scale: live players deliver frames at the new box.
        for want in &wants {
            if let Some(live) = self.videos.players.get_mut(&want.key)
                && live.box_px != Some(want.box_px)
            {
                live.player.set_frame_box(want.box_px);
                live.box_px = Some(want.box_px);
            }
        }
        let jobs = self.videos.store.want(wants);
        self.videos.prune_surfaces(&wanted);
        self.queue_video_probes(jobs);
        self.reconcile_videos();
    }

    /// Starts `job` (the read slot is its, `media_reads`): it fetches its video through the
    /// SDK's cache and probes it off the UI thread, then lands in the store. Returns whether it
    /// started.
    pub(super) fn start_video_probe(&mut self, job: ProbeJob) -> bool {
        let Some(sdk) = self.sdk.clone() else {
            self.videos.store.probe_dropped(job.key);
            return false;
        };
        {
            let engine = Arc::clone(&self.videos.engine);
            let updates = sdk.updates();
            sdk.run(move |client| async move {
                let read = async move {
                    let fetched = match client.session(job.key.account) {
                        Ok(session) => session
                            .media()
                            .fetch(MediaSource::Attachment(job.key.attachment), false)
                            .await
                            .ok(),
                        Err(_) => None,
                    };
                    match fetched {
                        None => Err(crate::playback::PlaybackFailure::Broken),
                        Some(data) => {
                            let bytes = data.value.bytes;
                            tokio::task::spawn_blocking(move || engine.probe(bytes, job.box_px))
                                .await
                                .unwrap_or(Err(crate::playback::PlaybackFailure::Unavailable))
                        }
                    }
                };
                // One read at a time: a stalled one must not hold every other back.
                let outcome = tokio::time::timeout(super::media_reads::READ_DEADLINE, read)
                    .await
                    .unwrap_or(Err(crate::playback::PlaybackFailure::Broken));
                updates.send(move |shell| {
                    shell.video_probed(job.key, outcome);
                    true
                });
            });
        }
        true
    }

    /// Video `key`'s probe ended.
    pub(super) fn video_probed(
        &mut self,
        key: MediaKey,
        outcome: Result<crate::playback::Probe, crate::playback::PlaybackFailure>,
    ) {
        let outcome = outcome.map(|probe| {
            if let Some(poster) = probe.poster {
                let surface = self.videos.surface(key);
                if let Err(error) = surface.push_bytes(byte_frame(poster), 0) {
                    log::debug!("A video's first frame wasn't drawn: {error}");
                }
            }
            probe.info
        });
        let jobs = self.videos.store.probed(key, outcome);
        // Queued first, so the next video goes before any audio.
        self.queue_video_probes(jobs);
        self.media_read_done();
        self.reconcile_videos();
    }

    /// Opens, steers and lets go of players as the store wants them now.
    pub(super) fn reconcile_videos(&mut self) {
        // What plays may change: the clock is read at the next poll.
        self.videos.next_clock_s = None;
        // Autoplay videos as set now (a switch in Settings takes effect at once).
        let autoplay = self
            .conversations
            .settings()
            .autoplays_videos(self.videos.reduced_motion);
        self.videos.store.set_autoplay(autoplay);
        let desired = self.videos.store.desired();
        let gone: Vec<MediaKey> = self
            .videos
            .players
            .keys()
            .filter(|key| !desired.contains_key(key))
            .copied()
            .collect();
        for key in gone {
            if let Some(live) = self.videos.players.remove(&key) {
                let position_ms = live.player.status().position_ms;
                self.videos.store.released(key, position_ms);
                self.videos.positions.remove(&key);
            }
        }
        self.videos
            .opening
            .retain(|key, _| desired.contains_key(key));
        self.videos
            .opening_box
            .retain(|key, _| desired.contains_key(key));
        let volume = self.videos.volume.get();
        for (key, want) in desired {
            if let Some(live) = self.videos.players.get_mut(&key) {
                steer(live, want);
            } else if !self.videos.opening.contains_key(&key) {
                self.open_video(key, want, volume);
            }
        }
    }

    /// Opens a player for video `key`, from its original in the SDK's cache.
    fn open_video(&mut self, key: MediaKey, want: Desired, volume: f32) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        self.videos.serial += 1;
        let serial = self.videos.serial;
        self.videos.opening.insert(key, serial);
        let surface = self.videos.surface(key);
        let engine = Arc::clone(&self.videos.engine);
        let frame_px = self.video_box_px(key);
        self.videos.opening_box.insert(key, frame_px);
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
                changed_updates.send(move |shell| shell.video_changed(key, serial));
            });
            let player = bytes.map(|bytes| {
                engine.open(
                    bytes,
                    PlayerSetup {
                        frame_px,
                        playing: want.playing,
                        start_ms: want.start_ms,
                        muted: want.muted,
                        volume,
                        looping: want.looping,
                    },
                    Some(Arc::new(SurfaceSink::new(surface))),
                    changed,
                )
            });
            updates.send(move |shell| {
                shell.video_opened(key, serial, want, player);
                true
            });
        });
    }

    /// A player for video `key` opened (or its bytes couldn't be read).
    fn video_opened(
        &mut self,
        key: MediaKey,
        serial: u64,
        applied: Desired,
        player: Option<Box<dyn Player>>,
    ) {
        if self.videos.opening.get(&key) != Some(&serial) {
            // No longer wanted, or superseded: dropping it stops it.
            return;
        }
        self.videos.opening.remove(&key);
        let box_px = self.videos.opening_box.remove(&key).flatten();
        if let Some(player) = player {
            self.videos.players.insert(
                key,
                Live {
                    player,
                    applied,
                    serial,
                    box_px,
                },
            );
        } else {
            self.videos
                .store
                .player_failed(key, crate::playback::PlaybackFailure::Broken);
        }
        // The store may have moved on while it opened.
        self.reconcile_videos();
    }

    /// Video `key`'s player (`serial`) changed: it ended or failed. Returns whether views
    /// change.
    fn video_changed(&mut self, key: MediaKey, serial: u64) -> bool {
        let Some(live) = self
            .videos
            .players
            .get(&key)
            .filter(|live| live.serial == serial)
        else {
            return false;
        };
        match live.player.status().state {
            PlayerState::Ended => self.videos.store.ended(key),
            PlayerState::Failed(failure) => self.videos.store.player_failed(key, failure),
            PlayerState::Loading | PlayerState::Paused | PlayerState::Playing => return true,
        }
        self.reconcile_videos();
        true
    }

    /// A press on video `key`'s controls.
    pub(super) fn press_video(
        &mut self,
        at: MediaAt,
        key: MediaKey,
        press: VideoPress,
        tx: &mut Tx,
    ) {
        match press {
            VideoPress::Play => self.videos.store.play(key),
            VideoPress::Pause => self.videos.store.pause(key),
            VideoPress::Sound(on) => self.videos.store.set_sound(key, on),
            VideoPress::Seek { share, done } => {
                let duration_ms = self
                    .videos
                    .store
                    .view()
                    .get(key)
                    .and_then(crate::video_store::VideoShown::duration_ms)
                    .unwrap_or_default();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "a share of a video's length"
                )]
                let position_ms = (f64::from(share.clamp(0.0, 1.0)) * duration_ms as f64) as u64;
                self.seek_video(key, position_ms, done);
            }
            VideoPress::SeekBy(by_ms) => {
                let video = self.videos.view();
                let Some(shown) = video.get(key) else {
                    return;
                };
                let end = shown.duration_ms().unwrap_or(u64::MAX);
                let position_ms = shown.position_ms.saturating_add_signed(by_ms).min(end);
                self.seek_video(key, position_ms, true);
            }
            VideoPress::Volume(volume) => {
                if let Some(write) = self.videos.volume.set(volume) {
                    self.write_video_volume(write, tx);
                }
                let volume = self.videos.volume.get();
                for live in self.videos.players.values() {
                    live.player.set_volume(volume);
                }
                if volume > 0.0 {
                    self.videos.store.set_sound(key, true);
                }
            }
            VideoPress::Viewer => {
                self.press_media(at, crate::message_media::received::MediaPress::Open, tx);
                return;
            }
            VideoPress::Retry => {
                let jobs = self.videos.store.retry(key);
                self.queue_video_probes(jobs);
            }
        }
        self.reconcile_videos();
        // One sound at a time: a video's sound pauses audio.
        if self.videos.store.has_sound() {
            self.pause_audio();
        }
    }

    /// Moves video `key` to `position_ms`: the live player now (exactly once `exact`), else
    /// where it opens.
    fn seek_video(&mut self, key: MediaKey, position_ms: u64, exact: bool) {
        self.videos.store.seeked(key, position_ms);
        if let Some(live) = self.videos.players.get_mut(&key) {
            let mode = if exact {
                SeekMode::Accurate
            } else {
                SeekMode::Keyframe
            };
            live.player.seek(position_ms, mode);
            // The store now wants it there: steering mustn't seek it again.
            live.applied.start_ms = position_ms;
            self.videos.positions.insert(key, position_ms);
        }
    }

    /// Video `key`'s box is at `rect`: which videos show may change. Returns whether views
    /// change.
    pub(super) fn video_moved(
        &mut self,
        key: MediaKey,
        rect: [f32; 4],
        reduced_motion: bool,
    ) -> bool {
        self.videos.rects.insert(key, rect);
        let autoplay_changed = self.videos.reduced_motion != reduced_motion;
        self.videos.reduced_motion = reduced_motion;
        if autoplay_changed {
            let autoplay = self
                .conversations
                .settings()
                .autoplays_videos(reduced_motion);
            self.videos.store.set_autoplay(autoplay);
        }
        self.show_videos()
    }

    /// Hands the store each video's shown share. Returns whether what plays changed.
    pub(super) fn show_videos(&mut self) -> bool {
        let span = self
            .tabs
            .shown()
            .and_then(|open| open.tracker.shown_span_pt());
        let shares: HashMap<MediaKey, f32> = match span {
            None => HashMap::new(),
            Some((top_pt, bottom_pt)) => self
                .videos
                .rects
                .iter()
                .map(|(key, rect)| {
                    (
                        *key,
                        super::received_media::shown_share(*rect, top_pt, bottom_pt),
                    )
                })
                .collect(),
        };
        let before = self.videos.store.desired();
        self.videos.store.shown(&shares);
        if self.videos.store.desired() == before {
            return false;
        }
        self.reconcile_videos();
        true
    }

    /// The window's clock: live players' positions for the controls. Returns whether views
    /// change, and asks for the next tick while a shown video plays.
    pub(super) fn poll_videos(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        if self.videos.players.is_empty() {
            self.videos.next_clock_s = None;
            return !std::mem::take(&mut self.videos.positions).is_empty();
        }
        // Every decoded frame wakes the window; the time is read only at the clock's pace.
        if self.videos.next_clock_s.is_some_and(|next| clock_s < next) {
            return false;
        }
        // Times are kept as shown: whole seconds at rest, quarter seconds for the seek bar, so
        // a playing row is drawn anew only when what it shows moves.
        let controls = self.videos.attention.is_some();
        let (step_ms, wait_s) = if controls {
            (CLOCK_STEP_MS, CLOCK_TICK_S)
        } else {
            (1_000, 1.0 - clock_s.fract())
        };
        let mut positions = HashMap::new();
        let mut playing = false;
        for (key, live) in &self.videos.players {
            let status = live.player.status();
            playing |= status.state == PlayerState::Playing;
            positions.insert(*key, status.position_ms / step_ms * step_ms);
        }
        let changed = positions != self.videos.positions;
        self.videos.positions = positions;
        self.videos.next_clock_s = playing.then(|| clock_s + wait_s.max(0.05));
        if playing {
            tx.wake_after(std::time::Duration::from_secs_f64(wait_s.max(0.05)));
        }
        changed
    }

    /// Video `key`'s box in device pixels, at the Video size.
    fn video_box_px(&self, key: MediaKey) -> Option<[u32; 2]> {
        let open = self.tabs.shown()?;
        let look = self.look(false);
        let bounds = self.video_bounds(&look);
        let attachment = open.items.iter().find_map(|item| match item.as_ref() {
            papo_sdk_ui::TimelineItem::Message { message, .. } => message
                .attachments
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|attachment| attachment_id(attachment) == Some(key.attachment))
                .cloned(),
            _ => None,
        })?;
        Some(box_px(&attachment, bounds, &self.own_sizes, self.scale))
    }
}

/// Tells `live` what `want` changed.
fn steer(live: &mut Live, want: Desired) {
    let applied = live.applied;
    if want.muted != applied.muted {
        live.player.set_muted(want.muted);
    }
    if want.looping != applied.looping {
        live.player.set_looping(want.looping);
    }
    // A seek made while its player was opening reaches it now.
    if want.start_ms != applied.start_ms {
        live.player.seek(want.start_ms, SeekMode::Accurate);
    }
    if want.playing != applied.playing {
        if want.playing {
            live.player.play();
        } else {
            live.player.pause();
        }
    }
    live.applied = want;
}

/// A video's box in device pixels.
fn box_px(
    attachment: &Attachment,
    bounds: MediaBounds,
    own_sizes: &HashMap<papo_sdk::Uuid, papo_sdk_ui::MediaSize>,
    scale: f32,
) -> [u32; 2] {
    let size = attachment_id(attachment)
        .and_then(|id| own_sizes.get(&id).copied())
        .unwrap_or_else(|| papo_sdk_ui::MediaSize::of(attachment));
    let boxed = media_box(size, bounds);
    [boxed.width_pt, boxed.height_pt].map(|side| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a box side in device pixels"
        )]
        let px = (side * scale).round().max(2.0) as u32;
        px
    })
}

/// Where a message's videos are drawn and how large: what `videos_wanted` reads.
pub(super) struct Wanting<'a> {
    pub(super) account: AccountId,
    pub(super) own_sizes: &'a HashMap<papo_sdk::Uuid, papo_sdk_ui::MediaSize>,
    pub(super) revealed: &'a std::collections::HashSet<MediaKey>,
    /// A video alone takes the Video size; an album's cells the album's box (Image size).
    pub(super) bounds: MediaBounds,
    pub(super) album_bounds: MediaBounds,
    pub(super) scale: f32,
    /// Drawn where videos never play (the pins panel): their first frame only.
    pub(super) still: bool,
}

/// What a message's videos want: each one's box in device pixels, and whether it may play
/// where it is drawn.
pub(super) fn videos_wanted(attachments: &[Attachment], wanting: &Wanting<'_>) -> Vec<VideoWant> {
    let (boxed, _) = split(attachments);
    let in_album = boxed.len() > 1;
    let shown = if in_album {
        papo_sdk_ui::AlbumLayout::for_count(boxed.len()).map_or(0, papo_sdk_ui::AlbumLayout::shown)
    } else {
        boxed.len()
    };
    boxed
        .into_iter()
        .take(shown)
        .filter_map(|index| {
            let attachment = &attachments[index];
            if AttachmentKind::of(attachment) != AttachmentKind::Video {
                return None;
            }
            let id = attachment_id(attachment)?;
            let bounds = if in_album {
                wanting.album_bounds
            } else {
                wanting.bounds
            };
            Some(VideoWant {
                key: MediaKey {
                    account: wanting.account,
                    attachment: id,
                },
                box_px: box_px(attachment, bounds, wanting.own_sizes, wanting.scale),
                in_album: in_album || wanting.still,
                hidden: is_hidden(wanting.account, attachment, wanting.revealed),
            })
        })
        .collect()
}

impl Shell {
    /// The largest box a video takes at the Video size, in device pixels: a local video's first
    /// frame is read to fit it, for its tile and its unsent message.
    pub(super) fn video_probe_box_px(&self) -> [u32; 2] {
        let bounds = self.video_bounds(&self.look(false));
        [bounds.max_width_pt, bounds.max_height_pt].map(|side| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a box side in device pixels"
            )]
            let px = (side * self.scale).round().max(2.0) as u32;
            px
        })
    }
}

/// A staged video read for its tile and its unsent message (blocking work on a worker): its
/// first frame fitted into `box_px` on a surface of its own, its length and size. None when it
/// can't be read or played here; it then draws as a box without a frame. One at a time: each
/// may read up to 100 MiB.
pub(super) async fn read_local_video(
    session: &papo_sdk::Session,
    staged: &papo_sdk::StagedAttachment,
    engine: Arc<dyn Engine>,
    box_px: [u32; 2],
) -> Option<crate::composer::tray::LocalVideo> {
    static ONE_AT_A_TIME: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let _turn = ONE_AT_A_TIME.acquire().await.ok()?;
    let bytes =
        super::staged_files::read_staged(session, staged, papo_sdk::MAX_ATTACHMENT_BYTES).await?;
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let probe = tokio::task::spawn_blocking(move || engine.probe(bytes, box_px))
        .await
        .ok()?
        .ok()?;
    let surface = NativeSurface::new();
    if let Some(poster) = probe.poster
        && let Err(error) = surface.push_bytes(byte_frame(poster), 0)
    {
        log::debug!("A local video's first frame wasn't drawn: {error}");
    }
    Some(crate::composer::tray::LocalVideo {
        surface,
        duration_ms: probe.info.duration_ms,
        size_px: probe.info.video_px,
    })
}

impl Shell {
    /// A video key on focused message `id` of timeline `key` (Christian, 2026-11-25): Space or
    /// K plays or pauses its video, M turns its sound on or off, as on the video itself; F opens
    /// it in the viewer in full screen. Only a video alone in its message plays in place (an
    /// album's never do, §14.4); F opens either. Returns whether it took the key.
    pub(super) fn message_video_key(
        &mut self,
        key: crate::timeline::TimelineKey,
        id: papo_sdk::Uuid,
        typed: &str,
        tx: &mut Tx,
    ) -> bool {
        let at = MediaAt {
            surface: crate::message_media::received::MediaSurface::Timeline,
            account: key.account,
            channel: key.channel,
            message: id,
            index: 0,
        };
        let Some(message) = self.message_at(at) else {
            return false;
        };
        let attachments = message.attachments.as_deref().unwrap_or_default();
        let (boxed, _) = split(attachments);
        let Some((index, video)) = boxed.iter().find_map(|index| {
            let attachment = &attachments[*index];
            (AttachmentKind::of(attachment) == AttachmentKind::Video)
                .then(|| attachment_id(attachment).map(|id| (*index, id)))
                .flatten()
        }) else {
            return false;
        };
        let at = MediaAt { index, ..at };
        let video = MediaKey {
            account: key.account,
            attachment: video,
        };
        let alone = boxed.len() == 1;
        let shown = self.videos.view().get(video).copied();
        let press = match typed {
            "f" | "F" => {
                self.open_viewer(at, tx);
                if let Some(viewer) = self.viewer.as_mut().filter(|viewer| !viewer.full_screen) {
                    viewer.full_screen = true;
                    tx.toggle_full_screen();
                }
                return true;
            }
            " " | "k" | "K" if alone => match shown {
                Some(shown) if shown.playing => VideoPress::Pause,
                Some(_) => VideoPress::Play,
                None => return false,
            },
            "m" | "M" if alone => match shown {
                Some(shown) => VideoPress::Sound(shown.muted),
                None => return false,
            },
            _ => return false,
        };
        self.press_video(at, video, press, tx);
        true
    }
}
