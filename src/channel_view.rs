//! Uma única superfície isolada de app, ao lado do ActiveEmbed existente.
use crate::webembed::{EmbedViewport, WebEmbedInput};
use papo_core::api::models::AppSurface;
use std::time::{Duration, Instant};

pub enum ViewEvent {
    Loading,
    Ready,
    Failed,
    External(String),
}
pub trait ViewBackend {
    fn create(&mut self, url: &str, bundled: bool) -> Result<(), String>;
    fn prepare(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context);
    fn present(&mut self, viewport: &EmbedViewport);
    fn resize_loading(&mut self, _viewport: &EmbedViewport) {}
    fn hide(&mut self);
    fn destroy(&mut self);
    fn poll(&mut self) -> Vec<ViewEvent>;
    fn texture_id(&self) -> Option<egui::TextureId> {
        None
    }
    fn texture_size(&self) -> Option<(u32, u32)> {
        None
    }
    fn input(&mut self, _input: WebEmbedInput) {}
    fn keyboard(&mut self, _events: &[egui::Event], _modifiers: egui::Modifiers, _focused: bool) {}
}

pub struct ChannelViewSurface {
    backend: Option<Box<dyn ViewBackend>>,
    owner: (String, String),
    selected: Option<(String, AppSurface, String)>,
    started: Option<Instant>,
    ready: bool,
    failed: bool,
    presented: bool,
    inline_seen: bool,
    external: Vec<String>,
}
impl Default for ChannelViewSurface {
    fn default() -> Self {
        Self {
            backend: platform_backend(),
            owner: Default::default(),
            selected: None,
            started: None,
            ready: false,
            failed: false,
            presented: false,
            inline_seen: false,
            external: Vec::new(),
        }
    }
}
impl ChannelViewSurface {
    pub fn owns(&self, server: &str, channel: &str) -> bool {
        self.owner.0 == server && self.owner.1 == channel
    }
    pub fn installation_id(&self) -> Option<&str> {
        self.selected
            .as_ref()
            .map(|v| v.1.app_installation_id.as_str())
    }
    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_ref().map(|v| v.0.as_str())
    }
    pub fn is_fallback(&self) -> bool {
        self.failed
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }
    pub fn url(&self) -> Option<&str> {
        self.selected
            .as_ref()
            .map(|v| v.2.as_str())
            .filter(|s| !s.is_empty())
    }
    pub fn sync(&mut self, server: &str, channel: &str) {
        if !self.owns(server, channel) {
            self.destroy();
            self.owner = (server.to_owned(), channel.to_owned());
        }
    }
    #[cfg(test)]
    pub fn activate(&mut self, id: String, app: AppSurface, server: &str) {
        self.activate_ticket(id, app, server, "test");
    }
    pub fn activate_ticket(&mut self, id: String, app: AppSurface, server: &str, ticket: &str) {
        if self
            .selected
            .as_ref()
            .is_some_and(|v| v.0 == id && v.1 == app)
        {
            return;
        }
        self.destroy();
        let url = surface_url(&app, server);
        self.failed = url.is_none() || !app.available;
        let mut url = url.unwrap_or_default();
        if !self.failed
            && let Ok(mut parsed) = url::Url::parse(&url)
        {
            parsed.set_fragment(Some(&format!(
                "papo_ticket={}",
                url::form_urlencoded::byte_serialize(ticket.as_bytes()).collect::<String>()
            )));
            url = parsed.to_string();
        }
        if !self.failed {
            self.failed = match self.backend.as_mut() {
                Some(b) => b
                    .create(&url, app.source == "bundled")
                    .map_err(|e| log::warn!("app surface: {e}"))
                    .is_err(),
                None => true,
            };
            if !self.failed {
                self.started = Some(Instant::now());
            }
        }
        self.selected = Some((id, app, url));
    }
    pub fn begin_frame(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context) {
        self.presented = false;
        self.inline_seen = false;
        if let Some(b) = &mut self.backend {
            b.prepare(frame, ctx);
        }
        self.pump();
        if self.started.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn pump(&mut self) {
        if self.failed || self.selected.is_none() {
            return;
        }
        if let Some(b) = &mut self.backend {
            for event in b.poll() {
                match event {
                    ViewEvent::Loading => {
                        if self.started.is_none() {
                            self.started = Some(Instant::now());
                        }
                        self.ready = false;
                    }
                    ViewEvent::Ready => {
                        self.started = None;
                        self.ready = true;
                    }
                    ViewEvent::Failed => self.failed = true,
                    ViewEvent::External(url) => self.external.push(url),
                }
            }
        }
        if self
            .started
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(15))
        {
            self.failed = true;
        }
        if self.failed {
            if let Some(b) = &mut self.backend {
                b.destroy();
            }
            self.started = None;
            self.ready = false;
        }
    }
    pub fn present(&mut self, viewport: &EmbedViewport) {
        self.pump();
        if !self.failed && self.selected.is_some() {
            // Hidden while loading; WPE still needs its real dimensions to render.
            if let Some(b) = &mut self.backend {
                if self.ready {
                    b.present(viewport);
                } else {
                    b.resize_loading(viewport);
                }
            }
            self.presented = true;
        }
    }
    pub fn present_inline(
        &mut self,
        rect: egui::Rect,
        clip: egui::Rect,
        ppp: f32,
        occlusions: &[egui::Rect],
        allowed: bool,
    ) {
        let clipped = rect.intersect(clip);
        if !allowed {
            self.hide();
            return;
        }
        if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
            self.destroy();
            return;
        }
        if let Some(viewport) =
            crate::webembed::inline_viewport(rect, clip, ppp, occlusions, allowed)
        {
            self.present(&viewport);
        }
    }
    pub fn observe_inline(&mut self, rect: egui::Rect, clip: egui::Rect) {
        let r = rect.intersect(clip);
        self.inline_seen = r.width() > 0.5 && r.height() > 0.5;
    }
    pub fn hide(&mut self) {
        if let Some(b) = &mut self.backend {
            b.hide();
        }
    }
    pub fn end_frame(&mut self) {
        if !self.presented {
            if self
                .selected_id()
                .is_some_and(|id| id.starts_with("inline-app:") || id.starts_with("button-app:"))
                && !self.inline_seen
            {
                self.destroy();
            } else {
                self.hide();
            }
        }
    }
    pub fn destroy(&mut self) {
        if let Some(id) = self.selected_id() {
            log::debug!("app surface: destroy {id}");
        }
        if let Some(b) = &mut self.backend {
            b.destroy();
        }
        self.selected = None;
        self.external.clear();
        self.started = None;
        self.ready = false;
        self.failed = false;
    }
    pub fn take_external(&mut self) -> Vec<String> {
        std::mem::take(&mut self.external)
    }
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        if self.ready {
            self.backend.as_ref().and_then(|b| b.texture_id())
        } else {
            None
        }
    }
    pub fn texture_size(&self) -> Option<(u32, u32)> {
        self.backend.as_ref().and_then(|b| b.texture_size())
    }
    pub fn keyboard(&mut self, events: &[egui::Event], modifiers: egui::Modifiers, focused: bool) {
        if self.ready
            && let Some(b) = &mut self.backend
        {
            b.keyboard(events, modifiers, focused);
        }
    }
    pub fn input(&mut self, input: WebEmbedInput) {
        if self.ready
            && let Some(b) = &mut self.backend
        {
            b.input(input);
        }
    }
}
impl Drop for ChannelViewSurface {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Only payload-backed app surfaces can use the Papo-host bundled exception.
pub fn surface_url(app: &AppSurface, server: &str) -> Option<String> {
    if app.source == "remote" {
        return valid_view_url(&app.entry, server).then(|| app.entry.clone());
    }
    if app.source != "bundled" || !valid_installation_id(&app.app_installation_id) {
        return None;
    }
    let base = url::Url::parse(server).ok()?;
    let url = base.join(&app.entry).ok()?;
    let prefix = format!("/apps/{}/ui/", app.app_installation_id);
    (matches!(url.scheme(), "https" | "http")
        && url.origin() == base.origin()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with(&prefix)
        && url.path().len() > prefix.len()
        && !url.path().contains('%')
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.to_string())
}
pub fn valid_view_url(raw: &str, server: &str) -> bool {
    let (Ok(url), Ok(server)) = (url::Url::parse(raw), url::Url::parse(server)) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.host_str() != server.host_str()
        && url.username().is_empty()
        && url.password().is_none()
        && papo_core::preview::safe_remote_url(raw)
}

/// Comparação de origem normalizada, incluindo a porta efetiva.
pub fn same_origin(first: &str, second: &str) -> bool {
    match (url::Url::parse(first), url::Url::parse(second)) {
        (Ok(a), Ok(b)) => matches!(a.scheme(), "http" | "https") && a.origin() == b.origin(),
        _ => false,
    }
}

/// Política conservadora exigida pelo contrato, aplicada só à resposta principal.
pub fn refuses_framing(xfo: &str, csp: &str) -> bool {
    if xfo.split(',').any(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "deny" | "sameorigin"
        )
    }) {
        return true;
    }
    csp.split(',')
        .flat_map(|policy| policy.split(';'))
        .any(|directive| {
            let mut words = directive.split_ascii_whitespace();
            words
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
                && words.collect::<Vec<_>>() != ["*"]
        })
}

#[cfg(target_os = "windows")]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    Some(Box::new(
        crate::platform::windows_webembed::channel_view::WindowsChannelView::new(),
    ))
}
#[cfg(target_os = "linux")]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    Some(Box::new(
        crate::platform::linux_webembed::LinuxAppSurface::default(),
    ))
}
#[cfg(target_os = "macos")]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    Some(Box::new(
        crate::platform::macos_webembed::MacosAppSurface::default(),
    ))
}
#[cfg(target_os = "android")]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    Some(Box::new(
        crate::platform::android_webembed::AndroidAppSurface::default(),
    ))
}
#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "android"
)))]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    None
}

fn valid_installation_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AppSurface {
        AppSurface {
            user_actions: vec![],
            app_installation_id: "11111111-1111-4111-8111-111111111111".into(),
            surface_id: "test".into(),
            source: "remote".into(),
            entry: "https://example.org".into(),
            avatar_url: String::new(),
            available: true,
        }
    }
    #[test]
    fn inline_fallback_survives_visible_frames_but_never_scrolls_into_a_float() {
        let mut surface = ChannelViewSurface::default();
        let _ = surface.backend.take();
        surface.activate(
            "inline-app:message:embed".into(),
            fixture(),
            "https://papo.example",
        );
        assert!(surface.is_fallback());
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(100.0));
        surface.observe_inline(rect, rect);
        surface.end_frame();
        assert!(surface.selected_id().is_some());
        surface.inline_seen = false;
        surface.end_frame();
        assert!(surface.selected_id().is_none());
    }
    #[test]
    fn launch_ticket_is_encoded_only_in_the_fragment() {
        let mut surface = ChannelViewSurface::default();
        let _ = surface.backend.take();
        surface.activate_ticket(
            "inline-app:m:e".into(),
            fixture(),
            "https://papo.example",
            "ticket?private=1&x#y",
        );
        let url = url::Url::parse(surface.url().unwrap()).unwrap();
        assert!(url.query().is_none());
        assert_eq!(
            url.fragment(),
            Some("papo_ticket=ticket%3Fprivate%3D1%26x%23y")
        );
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(100.0));
        surface.observe_inline(rect, rect);
        surface.hide();
        surface.end_frame();
        assert_eq!(surface.selected_id(), Some("inline-app:m:e"));
    }
    #[test]
    fn unavailable_and_timeout_fail_closed() {
        let mut surface = ChannelViewSurface::default();
        let _ = surface.backend.take();
        let mut app = fixture();
        app.available = false;
        surface.activate("channel-app:app".into(), app, "https://papo.example");
        assert!(surface.is_fallback());
        surface.failed = false;
        surface.started = Some(Instant::now() - Duration::from_secs(16));
        surface.pump();
        assert!(surface.is_fallback());
        assert!(!surface.is_ready());
        surface.sync("https://papo.example", "other");
        assert!(surface.selected_id().is_none());
    }
    #[test]
    fn payload_path_exception_is_narrow() {
        let mut app = AppSurface {
            user_actions: vec![],
            app_installation_id: "11111111-1111-4111-8111-111111111111".into(),
            surface_id: "test".into(),
            source: "bundled".into(),
            entry: "/apps/11111111-1111-4111-8111-111111111111/ui/index.html".into(),
            avatar_url: String::new(),
            available: true,
        };
        assert!(surface_url(&app, "http://localhost:8080").is_some());
        for entry in [
            "/auth/whoami",
            "/apps/11111111-1111-4111-8111-111111111111/ui/../../api",
            "https://evil.example/apps/11111111-1111-4111-8111-111111111111/ui/a",
        ] {
            app.entry = entry.into();
            assert!(surface_url(&app, "https://papo.example").is_none());
        }
        assert!(refuses_framing("DENY", ""));
        assert!(refuses_framing("", "frame-ancestors 'self'"));
        assert!(!refuses_framing("", "frame-ancestors *"));
        assert!(!same_origin(
            "https://example.com",
            "https://example.com:444"
        ));
    }
}
