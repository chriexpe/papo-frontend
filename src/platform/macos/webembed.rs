//! WKWebView backend for macOS. WebKit owns native input and drawing;
//! Rust owns the same activation, clipping and lifetime policies as Windows.
#![cfg(target_os = "macos")]

use crate::webembed::{EmbedViewport, WebEmbedBackend, WebEmbedEvent};
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_void},
    rc::{Rc, Weak},
};

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

unsafe extern "C" {
    fn papo_wk_create(
        parent: *mut c_void,
        url: *const c_char,
        ctx: *mut c_void,
        callback: extern "C" fn(*mut c_void, i32, *const c_char),
    ) -> *mut c_void;
    fn papo_wk_create_app(
        parent: *mut c_void,
        url: *const c_char,
        ctx: *mut c_void,
        callback: extern "C" fn(*mut c_void, i32, *const c_char),
        bundled: i32,
    ) -> *mut c_void;
    fn papo_wk_present(
        host: *mut c_void,
        full: NativeRect,
        visible: NativeRect,
        cuts: *const NativeRect,
        count: usize,
    );
    fn papo_wk_hide(host: *mut c_void);
    fn papo_wk_is_playing(host: *mut c_void) -> i32;
    fn papo_wk_destroy(host: *mut c_void);
}

fn native_rect(rect: egui::Rect, scale: f32) -> NativeRect {
    let s = scale.max(0.1) as f64;
    NativeRect {
        x: (rect.min.x as f64 * s).round(),
        y: (rect.min.y as f64 * s).round(),
        width: (rect.width() as f64 * s).round().max(1.0),
        height: (rect.height() as f64 * s).round().max(1.0),
    }
}

#[derive(Default)]
struct EventSink {
    id: String,
    events: RefCell<Vec<WebEmbedEvent>>,
    ctx: RefCell<Option<egui::Context>>,
    app_events: RefCell<Vec<crate::channel_view::ViewEvent>>,
}

extern "C" fn event_callback(raw: *mut c_void, kind: i32, url: *const c_char) {
    if raw.is_null() {
        return;
    }
    // SAFETY: the retained Rc pointer stays allocated until the host and its
    // navigation delegates are torn down on the same UI thread.
    let sink = unsafe { &*(raw as *const EventSink) };
    if kind == 3 {
        sink.app_events
            .borrow_mut()
            .push(crate::channel_view::ViewEvent::Ready);
        if let Some(ctx) = sink.ctx.borrow().as_ref() {
            ctx.request_repaint();
        }
        return;
    }
    let id = sink.id.clone();
    let event = match kind {
        1 if !url.is_null() => {
            let value = unsafe { CStr::from_ptr(url) }
                .to_string_lossy()
                .into_owned();
            WebEmbedEvent::OpenExternal { id, url: value }
        }
        2 => WebEmbedEvent::Failed { id },
        _ => return,
    };
    sink.events.borrow_mut().push(event);
    if let Some(ctx) = sink.ctx.borrow().as_ref() {
        ctx.request_repaint();
    }
}

struct LiveView {
    handle: *mut c_void,
    _callback_owner: Rc<EventSink>,
}
impl LiveView {
    fn new(
        parent: *mut c_void,
        url: &str,
        sink: Rc<EventSink>,
        app: Option<bool>,
    ) -> Result<Rc<Self>, String> {
        let url = CString::new(url).map_err(|_| "embedded URL contains NUL")?;
        let raw_owner = Rc::into_raw(sink.clone()) as *mut c_void;
        let handle = unsafe {
            match app {
                Some(bundled) => papo_wk_create_app(
                    parent,
                    url.as_ptr(),
                    raw_owner,
                    event_callback,
                    i32::from(bundled),
                ),
                None => papo_wk_create(parent, url.as_ptr(), raw_owner, event_callback),
            }
        };
        if handle.is_null() {
            unsafe {
                drop(Rc::from_raw(raw_owner as *const EventSink));
            }
            return Err("WKWebView requires the native AppKit window on the main thread".into());
        }
        // The callback pointer is an additional strong Rc owner released by Drop.
        Ok(Rc::new(Self {
            handle,
            _callback_owner: sink,
        }))
    }
    fn present(&self, viewport: &EmbedViewport) {
        let scale = viewport.pixels_per_point;
        let visible = viewport.rect.intersect(viewport.clip_rect);
        if visible.width() <= 0.5 || visible.height() <= 0.5 {
            unsafe {
                papo_wk_hide(self.handle);
            }
            return;
        }
        let cuts: Vec<_> = viewport
            .occlusions
            .iter()
            .map(|r| r.intersect(visible))
            .filter(|r| r.width() > 0.5 && r.height() > 0.5)
            .map(|r| native_rect(r, scale))
            .collect();
        unsafe {
            papo_wk_present(
                self.handle,
                native_rect(viewport.rect, scale),
                native_rect(visible, scale),
                cuts.as_ptr(),
                cuts.len(),
            );
        }
    }
}
impl Drop for LiveView {
    fn drop(&mut self) {
        unsafe {
            papo_wk_destroy(self.handle);
            // Balance Rc::into_raw in new(), after delegates are removed.
            // No native callbacks can outlive this host.
            // _callback_owner is a separate, ordinary strong reference.
            // Use the same stable pointer from the retained sink.
            drop(Rc::from_raw(Rc::as_ptr(&self._callback_owner)));
        }
    }
}

thread_local! {
    static GEOMETRY: RefCell<Option<(Weak<LiveView>, EmbedViewport)>> = const { RefCell::new(None) };
}
fn flush(viewport: egui::ViewportId) {
    if viewport != egui::ViewportId::ROOT {
        return;
    }
    let geometry = GEOMETRY.with(|slot| slot.borrow_mut().take());
    if let Some((live, geometry)) = geometry
        && let Some(live) = live.upgrade()
    {
        live.present(&geometry);
    }
}
fn clear_geometry() {
    GEOMETRY.with(|slot| {
        slot.borrow_mut().take();
    });
}

pub struct MacosWebEmbedBackend {
    parent: Option<*mut c_void>,
    app: Option<bool>,
    id: Option<String>,
    url: Option<String>,
    live: Option<Rc<LiveView>>,
    sink: Option<Rc<EventSink>>,
    ctx: Option<egui::Context>,
}

impl MacosWebEmbedBackend {
    pub fn new() -> Self {
        eframe::egui_wgpu::set_post_present_hook(flush);
        Self {
            app: None,
            parent: None,
            id: None,
            url: None,
            live: None,
            sink: None,
            ctx: None,
        }
    }
    fn ensure_live(&mut self) -> Result<(), String> {
        if self.live.is_some() {
            return Ok(());
        }
        let parent = self.parent.ok_or("AppKit window not captured yet")?;
        let id = self.id.as_ref().ok_or("no active WebEmbed")?.clone();
        let url = self.url.as_ref().ok_or("no WebEmbed URL")?.clone();
        let sink = Rc::new(EventSink {
            id,
            events: RefCell::new(Vec::new()),
            ctx: RefCell::new(self.ctx.clone()),
            app_events: RefCell::new(Vec::new()),
        });
        let live = LiveView::new(parent, &url, sink.clone(), self.app)?;
        self.sink = Some(sink);
        self.live = Some(live);
        Ok(())
    }
}
impl Default for MacosWebEmbedBackend {
    fn default() -> Self {
        Self::new()
    }
}
impl WebEmbedBackend for MacosWebEmbedBackend {
    fn create(&mut self, id: &str, url: &str) -> Result<(), String> {
        self.destroy_current();
        self.id = Some(id.into());
        self.url = Some(url.into());
        if self.parent.is_some() {
            self.ensure_live()?;
        }
        Ok(())
    }
    fn present(&mut self, id: &str, viewport: &EmbedViewport) {
        if self.id.as_deref() != Some(id) {
            return;
        }
        if let Err(error) = self.ensure_live() {
            log::warn!("webembed(wkwebview): {error}");
            if let Some(sink) = &self.sink {
                sink.events
                    .borrow_mut()
                    .push(WebEmbedEvent::Failed { id: id.into() });
            }
            return;
        }
        if let Some(live) = &self.live {
            GEOMETRY.with(|pending| pending.replace(Some((Rc::downgrade(live), viewport.clone()))));
        }
    }
    fn suspend(&mut self, id: &str) {
        if self.id.as_deref() == Some(id) {
            clear_geometry();
            if let Some(live) = &self.live {
                unsafe {
                    papo_wk_hide(live.handle);
                }
            }
        }
    }
    fn resume(&mut self, _id: &str) {
        // Visibility is restored only at post-present geometry, not stale coordinates.
    }
    fn destroy(&mut self, id: &str) {
        if self.id.as_deref() == Some(id) {
            self.destroy_current();
        }
    }
    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        self.sink
            .as_ref()
            .map(|s| std::mem::take(&mut *s.events.borrow_mut()))
            .unwrap_or_default()
    }
    fn set_context(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
        if let Some(s) = &self.sink {
            s.ctx.replace(Some(ctx.clone()));
        }
    }
    fn prepare_render(&mut self, frame: &mut eframe::Frame) {
        if self.parent.is_some() {
            return;
        }
        if let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            self.parent = Some(appkit.ns_view.as_ptr());
        }
    }
    fn is_playing(&self, id: &str) -> Option<bool> {
        if self.id.as_deref() != Some(id) {
            return None;
        }
        let result = unsafe { papo_wk_is_playing(self.live.as_ref()?.handle) };
        if result == -1 {
            None
        } else {
            Some(result != 0)
        }
    }
}
impl MacosWebEmbedBackend {
    fn destroy_current(&mut self) {
        clear_geometry();
        self.live = None;
        self.sink = None;
        self.id = None;
        self.url = None;
    }
}
impl Drop for MacosWebEmbedBackend {
    fn drop(&mut self) {
        self.destroy_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_conversion_is_scaled_once() {
        let r = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 50.0));
        let native = native_rect(r, 2.0);
        assert_eq!(
            (native.x, native.y, native.width, native.height),
            (20.0, 40.0, 200.0, 100.0)
        );
    }
}

/// App surfaces share only geometry with rich embeds; policy is selected natively.
pub struct MacosAppSurface(MacosWebEmbedBackend);
impl Default for MacosAppSurface {
    fn default() -> Self {
        Self(MacosWebEmbedBackend::new())
    }
}
impl crate::channel_view::ViewBackend for MacosAppSurface {
    fn create(&mut self, url: &str, bundled: bool) -> Result<(), String> {
        self.0.app = Some(bundled);
        self.0.create("app", url)
    }
    fn prepare(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context) {
        self.0.set_context(ctx);
        self.0.prepare_render(frame);
        if self.0.id.is_some()
            && self.0.live.is_none()
            && let Err(error) = self.0.ensure_live()
        {
            log::warn!("app surface (WKWebView): {error}");
            // No native callback exists when construction fails (e.g. OS <12).
            self.0.sink = Some(Rc::new(EventSink {
                app_events: RefCell::new(vec![crate::channel_view::ViewEvent::Failed]),
                ..Default::default()
            }));
        }
    }
    fn resize_loading(&mut self, viewport: &EmbedViewport) {
        self.0.present("app", viewport);
    }
    fn present(&mut self, viewport: &EmbedViewport) {
        self.0.present("app", viewport);
    }
    fn hide(&mut self) {
        self.0.suspend("app");
    }
    fn destroy(&mut self) {
        self.0.destroy_current();
    }
    fn poll(&mut self) -> Vec<crate::channel_view::ViewEvent> {
        use crate::channel_view::ViewEvent;
        let mut events = self
            .0
            .sink
            .as_ref()
            .map(|s| std::mem::take(&mut *s.app_events.borrow_mut()))
            .unwrap_or_default();
        for event in self.0.poll_events() {
            match event {
                WebEmbedEvent::Failed { .. } => events.push(ViewEvent::Failed),
                WebEmbedEvent::OpenExternal { url, .. } => events.push(ViewEvent::External(url)),
                _ => {}
            }
        }
        events
    }
}
