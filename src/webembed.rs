//! Browser-backed rich embeds.
//!
//! Preview discovery stays in `papo-core::preview` and direct audio/video
//! stays in `MediaStore`. This module owns only live browser-surface policy:
//! explicit activation, instance identity, geometry, off-screen behavior and
//! bounded lifecycle.

use egui::Rect;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum OffscreenBehavior {
    Stop,
    #[default]
    Float,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmbedViewport {
    pub rect: Rect,
    pub clip_rect: Rect,
    pub pixels_per_point: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebEmbedEvent {
    OpenExternal { id: String, url: String },
    Failed { id: String },
    ScrollTimeline { id: String, delta_y_px: f32 },
}

pub trait WebEmbedBackend {
    fn create(&mut self, id: &str, url: &str) -> Result<(), String>;
    fn present(&mut self, id: &str, viewport: &EmbedViewport);
    fn suspend(&mut self, id: &str);
    fn resume(&mut self, id: &str);
    fn destroy(&mut self, id: &str);
    fn poll_events(&mut self) -> Vec<WebEmbedEvent>;
}

struct ActiveEmbed {
    id: String,
    url: String,
    suspended: bool,
}

pub struct WebEmbedManager {
    backend: Box<dyn WebEmbedBackend>,
    active: Option<ActiveEmbed>,
    owner_seen: bool,
    inline_visible: bool,
    presented: bool,
    scroll_delta_px: f32,
}

impl Default for WebEmbedManager {
    fn default() -> Self {
        Self::new(platform_backend())
    }
}

impl WebEmbedManager {
    pub fn new(backend: Box<dyn WebEmbedBackend>) -> Self {
        Self {
            backend,
            active: None,
            owner_seen: false,
            inline_visible: false,
            presented: false,
            scroll_delta_px: 0.0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.owner_seen = false;
        self.inline_visible = false;
        self.presented = false;
    }

    /// Explicit user activation. Merely rendering a preview never creates a
    /// browser surface.
    pub fn activate(&mut self, id: String, url: String) -> bool {
        if !papo_core::preview::safe_remote_url(&url) {
            return false;
        }

        if self
            .active
            .as_ref()
            .is_some_and(|active| active.id == id && active.url == url)
        {
            self.resume_active();
            return true;
        }

        self.destroy_active();
        if let Err(error) = self.backend.create(&id, &url) {
            log::warn!("webembed não pôde ser criado: {error}");
            return false;
        }

        self.active = Some(ActiveEmbed {
            id,
            url,
            suspended: false,
        });
        true
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_ref().map(|active| active.id.as_str())
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.active_id() == Some(id)
    }

    /// The card that owns the embed is still part of this frame. Its native
    /// browser is only placed inline when the card actually intersects the
    /// egui clip rectangle.
    pub fn present_inline(
        &mut self,
        id: &str,
        rect: Rect,
        clip_rect: Rect,
        pixels_per_point: f32,
        allowed: bool,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.id != id {
            return;
        }
        self.owner_seen = true;

        let clipped = rect.intersect(clip_rect);
        let visible = allowed && clipped.width() > 0.5 && clipped.height() > 0.5;
        self.inline_visible = visible;
        if !visible {
            return;
        }

        if active.suspended {
            self.backend.resume(id);
            active.suspended = false;
        }
        self.backend.present(
            id,
            &EmbedViewport {
                rect,
                clip_rect,
                pixels_per_point: pixels_per_point.max(0.1),
            },
        );
        self.presented = true;
    }

    /// Off-screen floating playback is allowed only while the original card
    /// still belongs to the current channel/frame. Channel/server switches
    /// therefore never teleport a player into an unrelated conversation.
    pub fn should_float(&self, behavior: OffscreenBehavior) -> bool {
        behavior == OffscreenBehavior::Float
            && self.active.is_some()
            && self.owner_seen
            && !self.inline_visible
    }

    pub fn present_floating(
        &mut self,
        rect: Rect,
        clip_rect: Rect,
        pixels_per_point: f32,
        allowed: bool,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if !allowed {
            return;
        }
        let clipped = rect.intersect(clip_rect);
        if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
            return;
        }

        if active.suspended {
            self.backend.resume(&active.id);
            active.suspended = false;
        }
        self.backend.present(
            &active.id,
            &EmbedViewport {
                rect,
                clip_rect,
                pixels_per_point: pixels_per_point.max(0.1),
            },
        );
        self.presented = true;
    }

    /// Called after egui has had a chance to draw the floating slot.
    pub fn end_frame(&mut self, behavior: OffscreenBehavior, occluded: bool) {
        if self.active.is_none() {
            return;
        }

        // The owning card disappeared entirely: channel/server/auth switch.
        if !self.owner_seen {
            self.destroy_active();
            return;
        }

        if occluded {
            self.suspend_active();
            return;
        }

        if self.presented {
            return;
        }

        match behavior {
            OffscreenBehavior::Stop => self.destroy_active(),
            OffscreenBehavior::Float => self.suspend_active(),
        }
    }

    pub fn suspend_active(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if !active.suspended {
            self.backend.suspend(&active.id);
            active.suspended = true;
        }
    }

    pub fn resume_active(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.suspended {
            self.backend.resume(&active.id);
            active.suspended = false;
        }
    }

    pub fn destroy_active(&mut self) {
        if let Some(active) = self.active.take() {
            self.backend.destroy(&active.id);
        }
        self.owner_seen = false;
        self.inline_visible = false;
        self.presented = false;
    }

    pub fn trim(&mut self, level: crate::media::TrimLevel) {
        let destroy = match level {
            crate::media::TrimLevel::Light | crate::media::TrimLevel::Moderate => {
                self.active.as_ref().is_some_and(|active| active.suspended)
            }
            crate::media::TrimLevel::Critical => self.active.is_some(),
        };
        if destroy {
            self.destroy_active();
        }
    }

    /// Drain platform events. Navigation is deliberately externalized here;
    /// vertical touch hand-off is accumulated for the chat ScrollArea.
    pub fn pump_events(&mut self, ctx: &egui::Context) {
        for event in self.backend.poll_events() {
            match event {
                WebEmbedEvent::OpenExternal { url, .. } => {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
                WebEmbedEvent::Failed { id } => {
                    if self.is_active(&id) {
                        self.destroy_active();
                    }
                }
                WebEmbedEvent::ScrollTimeline { id, delta_y_px } => {
                    if self.is_active(&id) {
                        self.scroll_delta_px += delta_y_px;
                    }
                }
            }
        }
    }

    pub fn take_scroll_delta_points(&mut self, pixels_per_point: f32) -> f32 {
        let delta = std::mem::take(&mut self.scroll_delta_px);
        delta / pixels_per_point.max(0.1)
    }
}

impl Drop for WebEmbedManager {
    fn drop(&mut self) {
        self.destroy_active();
    }
}

#[cfg(target_os = "android")]
fn platform_backend() -> Box<dyn WebEmbedBackend> {
    Box::new(crate::platform::android_webembed::AndroidWebEmbedBackend)
}

#[cfg(not(target_os = "android"))]
fn platform_backend() -> Box<dyn WebEmbedBackend> {
    Box::new(UnavailableBackend)
}

#[cfg(not(target_os = "android"))]
struct UnavailableBackend;

#[cfg(not(target_os = "android"))]
impl WebEmbedBackend for UnavailableBackend {
    fn create(&mut self, _id: &str, _url: &str) -> Result<(), String> {
        Err("WebEmbed ainda não tem backend nesta plataforma".to_owned())
    }
    fn present(&mut self, _id: &str, _viewport: &EmbedViewport) {}
    fn suspend(&mut self, _id: &str) {}
    fn resume(&mut self, _id: &str) {}
    fn destroy(&mut self, _id: &str) {}
    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Create(String),
        Present(String),
        Suspend(String),
        Resume(String),
        Destroy(String),
    }

    struct FakeBackend {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    impl WebEmbedBackend for FakeBackend {
        fn create(&mut self, id: &str, _url: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(Call::Create(id.to_owned()));
            Ok(())
        }
        fn present(&mut self, id: &str, _viewport: &EmbedViewport) {
            self.calls.lock().unwrap().push(Call::Present(id.to_owned()));
        }
        fn suspend(&mut self, id: &str) {
            self.calls.lock().unwrap().push(Call::Suspend(id.to_owned()));
        }
        fn resume(&mut self, id: &str) {
            self.calls.lock().unwrap().push(Call::Resume(id.to_owned()));
        }
        fn destroy(&mut self, id: &str) {
            self.calls.lock().unwrap().push(Call::Destroy(id.to_owned()));
        }
        fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
            Vec::new()
        }
    }

    fn manager() -> (WebEmbedManager, Arc<Mutex<Vec<Call>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend {
            calls: calls.clone(),
        };
        (WebEmbedManager::new(Box::new(backend)), calls)
    }

    fn rect() -> Rect {
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 60.0))
    }

    #[test]
    fn placeholder_does_not_create_browser() {
        let (_manager, calls) = manager();
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn second_activation_replaces_first_browser() {
        let (mut manager, calls) = manager();
        assert!(manager.activate("a".into(), "https://example.com/a".into()));
        assert!(manager.activate("b".into(), "https://example.com/b".into()));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::Create("a".into()),
                Call::Destroy("a".into()),
                Call::Create("b".into()),
            ]
        );
    }

    #[test]
    fn scrolling_past_stops_when_policy_is_stop() {
        let (mut manager, calls) = manager();
        manager.activate("a".into(), "https://example.com/a".into());
        manager.begin_frame();
        manager.present_inline("a", rect(), Rect::NOTHING, 1.0, true);
        manager.end_frame(OffscreenBehavior::Stop, false);
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&Call::Destroy("a".into()))
        );
    }

    #[test]
    fn scrolling_past_can_rehome_same_browser_in_float() {
        let (mut manager, calls) = manager();
        manager.activate("a".into(), "https://example.com/a".into());
        manager.begin_frame();
        manager.present_inline("a", rect(), Rect::NOTHING, 1.0, true);
        assert!(manager.should_float(OffscreenBehavior::Float));
        manager.present_floating(rect(), rect(), 1.0, true);
        manager.end_frame(OffscreenBehavior::Float, false);
        let calls = calls.lock().unwrap();
        assert!(!matches!(calls.last(), Some(Call::Destroy(_))));
        assert!(calls.contains(&Call::Present("a".into())));
    }

    #[test]
    fn missing_owner_destroys_even_with_float_policy() {
        let (mut manager, calls) = manager();
        manager.activate("a".into(), "https://example.com/a".into());
        manager.begin_frame();
        manager.end_frame(OffscreenBehavior::Float, false);
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&Call::Destroy("a".into()))
        );
    }

    #[test]
    fn critical_trim_releases_live_browser() {
        let (mut manager, calls) = manager();
        manager.activate("a".into(), "https://example.com/a".into());
        manager.trim(crate::media::TrimLevel::Critical);
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&Call::Destroy("a".into()))
        );
    }
}
