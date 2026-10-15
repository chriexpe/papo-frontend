//! Uma única view passiva de canal, independente dos embeds ricos.
use crate::webembed::EmbedViewport;
use papo_core::api::models::ChannelView;
use std::time::{Duration, Instant};

pub enum ViewEvent {
    Loading,
    Ready,
    Failed,
}

pub trait ViewBackend {
    fn create(&mut self, url: &str) -> Result<(), String>;
    fn prepare(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context);
    fn present(&mut self, viewport: &EmbedViewport);
    fn hide(&mut self);
    fn destroy(&mut self);
    fn poll(&mut self) -> Vec<ViewEvent>;
}

/// Estado de uma única view. Nenhum cliente HTTP autenticado entra nesta API.
pub struct ChannelViewSurface {
    backend: Option<Box<dyn ViewBackend>>,
    owner: (String, String),
    selected: Option<ChannelView>,
    started: Option<Instant>,
    ready: bool,
    failed: bool,
    presented: bool,
}
impl Default for ChannelViewSurface {
    fn default() -> Self {
        Self::with_backend(platform_backend())
    }
}
impl ChannelViewSurface {
    fn with_backend(backend: Option<Box<dyn ViewBackend>>) -> Self {
        Self {
            backend,
            owner: Default::default(),
            selected: None,
            started: None,
            ready: false,
            failed: false,
            presented: false,
        }
    }
    pub fn owns(&self, server: &str, channel: &str) -> bool {
        self.owner.0 == server && self.owner.1 == channel
    }
    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_ref().map(|v| v.id.as_str())
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
            .filter(|v| valid_view_url(&v.url, &self.owner.0))
            .map(|v| v.url.as_str())
    }
    /// Uma troca de canal/servidor ou revogação destrói a instância imediatamente.
    pub fn sync(&mut self, server: &str, channel: &str, views: &[ChannelView]) {
        if self.owner.0 != server || self.owner.1 != channel {
            self.destroy();
            self.owner = (server.to_owned(), channel.to_owned());
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !views.contains(selected))
        {
            self.destroy();
        }
    }
    pub fn select(&mut self, view: Option<ChannelView>, server: &str) {
        if self.selected == view {
            return;
        }
        // Chat hides the same instance; selecting another view replaces it.
        self.destroy();
        let Some(view) = view else {
            return;
        };
        let valid = valid_view_url(&view.url, server);
        self.selected = Some(view.clone());
        self.failed = !valid;
        if valid {
            self.failed = match self.backend.as_mut() {
                Some(backend) => match backend.create(&view.url) {
                    Ok(()) => false,
                    Err(error) => {
                        log::warn!("channel-view indisponível: {error}");
                        true
                    }
                },
                None => true,
            };
            if !self.failed {
                self.started = Some(Instant::now());
            }
        }
    }
    pub fn begin_frame(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context) {
        self.presented = false;
        if let Some(backend) = &mut self.backend {
            backend.prepare(frame, ctx);
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
        if let Some(backend) = &mut self.backend {
            for event in backend.poll() {
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
                }
            }
        }
        if self
            .started
            .is_some_and(|start| start.elapsed() >= Duration::from_secs(15))
        {
            self.failed = true;
        }
        if self.failed {
            if let Some(backend) = &mut self.backend {
                backend.destroy();
            }
            self.started = None;
            self.ready = false;
        }
    }
    pub fn present(&mut self, viewport: &EmbedViewport) {
        self.pump();
        if !self.failed && self.ready && self.selected.is_some() {
            if let Some(backend) = &mut self.backend {
                backend.present(viewport);
            }
            self.presented = true;
        }
    }
    pub fn hide(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.hide();
        }
    }
    pub fn end_frame(&mut self) {
        if !self.presented {
            self.hide();
        }
    }
    pub fn destroy(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.destroy();
        }
        self.selected = None;
        self.started = None;
        self.ready = false;
        self.failed = false;
    }
}
impl Drop for ChannelViewSurface {
    fn drop(&mut self) {
        self.destroy();
    }
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
        (Ok(a), Ok(b)) => a.scheme() == "https" && a.origin() == b.origin(),
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
#[cfg(not(target_os = "windows"))]
fn platform_backend() -> Option<Box<dyn ViewBackend>> {
    // Android: WebView storage belongs to the process, not the instance.
    // WPE 2.0: navigation actions expose no reliable main-frame flag.
    // WKWebView: public permission delegates do not cover all required grants
    // on supported macOS versions. Never silently weaken the policy.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_origin_only() {
        assert!(same_origin(
            "https://example.com:443/a",
            "https://EXAMPLE.com/b"
        ));
        for other in [
            "http://example.com",
            "https://example.com:444",
            "https://sub.example.com",
            "file:///tmp/a",
            "javascript:alert(1)",
        ] {
            assert!(!same_origin("https://example.com", other));
        }
    }
    #[test]
    fn response_policy_handles_multiple_headers_and_directives() {
        for (xfo, csp) in [
            ("DENY", ""),
            ("sameorigin", ""),
            ("deny, SAMEORIGIN", ""),
            ("", "frame-ancestors 'none'"),
            ("", "default-src *; FRAME-ANCESTORS 'self'"),
            ("", "frame-ancestors *; frame-ancestors https://example.com"),
            ("", "frame-ancestors * , frame-ancestors 'none'"),
        ] {
            assert!(refuses_framing(xfo, csp));
        }
        assert!(!refuses_framing(
            "",
            "default-src 'self'; frame-ancestors *"
        ));
        assert!(!refuses_framing("", ""));
    }
    #[test]
    fn urls_never_accept_papo_or_userinfo() {
        let server = "https://papo.example";
        assert!(valid_view_url("https://map.example", server));
        for raw in [
            "https://papo.example:444/a",
            "https://user:secret@map.example",
            "http://map.example",
            "https://localhost/a",
        ] {
            assert!(!valid_view_url(raw, server));
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    #[derive(Default)]
    struct Trace {
        calls: Vec<&'static str>,
        events: Vec<ViewEvent>,
    }
    struct Backend(Rc<RefCell<Trace>>);
    impl ViewBackend for Backend {
        fn create(&mut self, _: &str) -> Result<(), String> {
            self.0.borrow_mut().calls.push("create");
            Ok(())
        }
        fn prepare(&mut self, _: &mut eframe::Frame, _: &egui::Context) {}
        fn present(&mut self, _: &EmbedViewport) {
            self.0.borrow_mut().calls.push("present");
        }
        fn hide(&mut self) {
            self.0.borrow_mut().calls.push("hide");
        }
        fn destroy(&mut self) {
            self.0.borrow_mut().calls.push("destroy");
        }
        fn poll(&mut self) -> Vec<ViewEvent> {
            std::mem::take(&mut self.0.borrow_mut().events)
        }
    }
    fn view(id: &str) -> ChannelView {
        ChannelView {
            id: id.into(),
            app_installation_id: "app".into(),
            channel_id: "a".into(),
            url: "https://map.example/".into(),
            position: 0,
            name: Some("Map".into()),
        }
    }
    fn surface() -> (ChannelViewSurface, Rc<RefCell<Trace>>) {
        let trace = Rc::new(RefCell::new(Trace::default()));
        let mut surface = ChannelViewSurface::with_backend(Some(Box::new(Backend(trace.clone()))));
        surface.sync("https://papo.example", "a", &[view("map")]);
        trace.borrow_mut().calls.clear();
        (surface, trace)
    }
    #[test]
    fn inactive_tab_hides_but_reuses_instance() {
        let (mut surface, trace) = surface();
        surface.select(Some(view("map")), "https://papo.example");
        surface.end_frame();
        surface.select(Some(view("map")), "https://papo.example");
        assert_eq!(surface.selected_id(), Some("map"));
        assert_eq!(
            trace
                .borrow()
                .calls
                .iter()
                .filter(|c| **c == "create")
                .count(),
            1
        );
        assert_eq!(trace.borrow().calls.last(), Some(&"hide"));
    }
    #[test]
    fn revocation_and_channel_switch_destroy_even_when_hidden() {
        let (mut surface, trace) = surface();
        surface.select(Some(view("map")), "https://papo.example");
        surface.hide();
        surface.sync("https://papo.example", "a", &[]);
        assert_eq!(surface.selected_id(), None);
        assert_eq!(trace.borrow().calls.last(), Some(&"destroy"));
        surface.select(Some(view("map")), "https://papo.example");
        surface.sync("https://papo.example", "b", &[view("map")]);
        assert_eq!(surface.selected_id(), None);
        assert_eq!(trace.borrow().calls.last(), Some(&"destroy"));
    }
    #[test]
    fn timeout_and_native_failure_destroy_and_offer_fallback() {
        for timeout in [true, false] {
            let (mut surface, trace) = surface();
            surface.select(Some(view("map")), "https://papo.example");
            if timeout {
                surface.started = Some(Instant::now() - Duration::from_secs(16));
            } else {
                trace.borrow_mut().events.push(ViewEvent::Failed);
            }
            surface.pump();
            assert!(surface.is_fallback());
            assert_eq!(surface.url(), Some("https://map.example/"));
            assert_eq!(trace.borrow().calls.last(), Some(&"destroy"));
        }
    }
    #[test]
    fn redirects_do_not_extend_the_load_deadline() {
        let (mut surface, trace) = surface();
        surface.select(Some(view("map")), "https://papo.example");
        let start = Instant::now() - Duration::from_secs(16);
        surface.started = Some(start);
        trace.borrow_mut().events.push(ViewEvent::Loading);
        surface.pump();
        assert!(surface.is_fallback());
    }
    #[test]
    fn unavailable_platform_never_creates_browser() {
        let mut surface = ChannelViewSurface::with_backend(None);
        surface.sync("https://papo.example", "a", &[view("map")]);
        surface.select(Some(view("map")), "https://papo.example");
        assert!(surface.is_fallback());
        assert!(!surface.is_ready());
    }
}
