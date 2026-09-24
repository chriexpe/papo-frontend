use std::path::PathBuf;

use egui::TextureHandle;

use super::player;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectMediaSource {
    File(PathBuf),
    RemoteUri(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectMediaKind {
    Audio,
    Video,
}

pub trait PlaybackPlayer: Send {
    fn play(&mut self);
    fn pause(&mut self);
    fn toggle(&mut self);
    fn seek(&mut self, seconds: f64);
    fn set_muted(&mut self, muted: bool);
    fn is_playing(&self) -> bool;
    fn position(&self) -> f64;
    fn duration(&self) -> f64;
    fn aspect(&self) -> f32;
    fn error(&self) -> Option<String>;
    fn update(&mut self);
    fn frame<'a>(&'a mut self, ctx: &egui::Context) -> Option<&'a TextureHandle>;
}

pub struct DirectMediaPlayer {
    inner: Box<dyn PlaybackPlayer>,
    pub muted: bool,
}

impl DirectMediaPlayer {
    pub fn new(inner: Box<dyn PlaybackPlayer>) -> Self {
        Self { inner, muted: false }
    }

    pub fn play(&mut self) { self.inner.play(); }
    pub fn pause(&mut self) { self.inner.pause(); }
    pub fn toggle(&mut self) { self.inner.toggle(); }
    pub fn seek(&mut self, seconds: f64) { self.inner.seek(seconds); }
    pub fn is_playing(&self) -> bool { self.inner.is_playing() }
    pub fn position(&self) -> f64 { self.inner.position() }
    pub fn duration(&self) -> f64 { self.inner.duration() }
    pub fn aspect(&self) -> f32 { self.inner.aspect() }
    pub fn error(&self) -> Option<String> { self.inner.error() }
    pub fn update(&mut self) { self.inner.update(); }
    pub fn frame<'a>(&'a mut self, ctx: &egui::Context) -> Option<&'a TextureHandle> {
        self.inner.frame(ctx)
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.inner.set_muted(muted);
    }
}

pub trait PlaybackBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn open(
        &self,
        source: DirectMediaSource,
        kind: DirectMediaKind,
        repaint: egui::Context,
    ) -> Option<DirectMediaPlayer>;
}

#[derive(Default)]
pub struct GStreamerBackend;

impl PlaybackBackend for GStreamerBackend {
    fn name(&self) -> &'static str { "gstreamer" }

    fn open(
        &self,
        source: DirectMediaSource,
        kind: DirectMediaKind,
        repaint: egui::Context,
    ) -> Option<DirectMediaPlayer> {
        let video = matches!(kind, DirectMediaKind::Video);
        let player = match source {
            DirectMediaSource::File(path) => player::Player::open(&path, video, repaint),
            DirectMediaSource::RemoteUri(uri) => player::Player::open_uri(&uri, video, repaint),
        }?;
        Some(DirectMediaPlayer::new(Box::new(player)))
    }
}

impl PlaybackPlayer for player::Player {
    fn play(&mut self) { player::Player::play(self); }
    fn pause(&mut self) { player::Player::pause(self); }
    fn toggle(&mut self) { player::Player::toggle(self); }
    fn seek(&mut self, seconds: f64) { player::Player::seek(self, seconds); }
    fn set_muted(&mut self, muted: bool) { player::Player::set_muted(self, muted); }
    fn is_playing(&self) -> bool { player::Player::is_playing(self) }
    fn position(&self) -> f64 { player::Player::position(self) }
    fn duration(&self) -> f64 { player::Player::duration(self) }
    fn aspect(&self) -> f32 { player::Player::aspect(self) }
    fn error(&self) -> Option<String> { player::Player::error(self) }
    fn update(&mut self) { player::Player::update(self); }
    fn frame<'a>(&'a mut self, ctx: &egui::Context) -> Option<&'a TextureHandle> {
        player::Player::frame(self, ctx)
    }
}
