//! Native Windows WebEmbed backend using Edge WebView2 directly.
//!
//! The shared WebEmbed manager owns lifetime/policy. WebView2 owns the child
//! HWND, Chromium renderer and native input. Unlike Linux there is no texture
//! bridge and Papo never forwards mouse events into the browser manually.

#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::sync::mpsc;

use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::{BOOL, E_POINTER, HWND, RECT},
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    },
    core::Interface as _,
};
use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2Environment, ICoreWebView2, ICoreWebView2Controller,
        ICoreWebView2Environment, ICoreWebView2_8,
    },
};

use crate::webembed::{EmbedViewport, WebEmbedBackend, WebEmbedEvent};

struct LiveWebView {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

impl LiveWebView {
    fn new(parent: HWND, url: &str) -> Result<Self, String> {
        let environment = create_environment()?;
        let controller = create_controller(&environment, parent)?;
        let webview = unsafe { controller.CoreWebView2() }
            .map_err(|error| format!("CoreWebView2 indisponível: {error}"))?;

        unsafe {
            let settings = webview
                .Settings()
                .map_err(|error| format!("WebView2 settings: {error}"))?;
            settings
                .SetAreDevToolsEnabled(false)
                .map_err(|error| format!("desativar DevTools: {error}"))?;
            settings
                .SetAreDefaultContextMenusEnabled(false)
                .map_err(|error| format!("desativar menu WebView2: {error}"))?;
            controller
                .SetIsVisible(false)
                .map_err(|error| format!("ocultar WebView2 inicial: {error}"))?;

            // First vertical slice: direct native navigation. Request-level
            // Referer/security hooks are added in this PR before merge.
            let uri = CoTaskMemPWSTR::from(url);
            webview
                .Navigate(*uri.as_ref().as_pcwstr())
                .map_err(|error| format!("navegar WebView2: {error}"))?;
        }

        Ok(Self {
            controller,
            webview,
        })
    }

    fn set_bounds(&self, viewport: &EmbedViewport) -> Result<(), String> {
        let clipped = viewport.rect.intersect(viewport.clip_rect);
        if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
            return self.set_visible(false);
        }

        let scale = viewport.pixels_per_point.max(0.1);
        let rect = RECT {
            left: (clipped.min.x * scale).round() as i32,
            top: (clipped.min.y * scale).round() as i32,
            right: (clipped.max.x * scale).round() as i32,
            bottom: (clipped.max.y * scale).round() as i32,
        };

        unsafe {
            self.controller
                .SetBounds(rect)
                .map_err(|error| format!("posicionar WebView2: {error}"))?;
            self.controller
                .NotifyParentWindowPositionChanged()
                .map_err(|error| format!("notificar posição WebView2: {error}"))?;
            self.controller
                .SetIsVisible(true)
                .map_err(|error| format!("mostrar WebView2: {error}"))?;
        }
        Ok(())
    }

    fn set_visible(&self, visible: bool) -> Result<(), String> {
        unsafe {
            self.controller
                .SetIsVisible(visible)
                .map_err(|error| format!("visibilidade WebView2: {error}"))
        }
    }

    fn is_playing(&self) -> Option<bool> {
        let webview: ICoreWebView2_8 = self.webview.cast().ok()?;
        let mut playing = BOOL(0);
        unsafe { webview.IsDocumentPlayingAudio(&mut playing) }.ok()?;
        Some(playing.as_bool())
    }

    fn close(&self) {
        let _ = unsafe { self.controller.Close() };
    }
}

fn create_environment() -> Result<ICoreWebView2Environment, String> {
    let (tx, rx) = mpsc::channel();

    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(|handler| unsafe {
            CreateCoreWebView2Environment(&handler).map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, environment| {
            error_code?;
            tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .map_err(|_| windows::core::Error::from(E_POINTER))?;
            Ok(())
        }),
    )
    .map_err(|error| format!("inicializar ambiente WebView2: {error}"))?;

    rx.recv()
        .map_err(|_| "callback de ambiente WebView2 foi encerrado".to_owned())?
        .map_err(|error| format!("criar ambiente WebView2: {error}"))
}

fn create_controller(
    environment: &ICoreWebView2Environment,
    parent: HWND,
) -> Result<ICoreWebView2Controller, String> {
    let (tx, rx) = mpsc::channel();
    let environment = environment.clone();

    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            environment
                .CreateCoreWebView2Controller(parent, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, controller| {
            error_code?;
            tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .map_err(|_| windows::core::Error::from(E_POINTER))?;
            Ok(())
        }),
    )
    .map_err(|error| format!("inicializar controller WebView2: {error}"))?;

    rx.recv()
        .map_err(|_| "callback do controller WebView2 foi encerrado".to_owned())?
        .map_err(|error| format!("criar controller WebView2: {error}"))
}

pub struct WindowsWebEmbedBackend {
    parent: Option<HWND>,
    pending: Option<(String, String)>,
    live: Option<LiveWebView>,
    current_id: Option<String>,
    events: Vec<WebEmbedEvent>,
    com_initialized: bool,
}

impl WindowsWebEmbedBackend {
    pub fn new() -> Self {
        Self {
            parent: None,
            pending: None,
            live: None,
            current_id: None,
            events: Vec::new(),
            com_initialized: false,
        }
    }

    fn ensure_com(&mut self) -> Result<(), String> {
        if self.com_initialized {
            return Ok(());
        }
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|error| format!("COM STA para WebView2: {error}"))?;
        }
        self.com_initialized = true;
        Ok(())
    }

    fn ensure_live(&mut self) -> Result<(), String> {
        if self.live.is_some() {
            return Ok(());
        }
        let parent = self
            .parent
            .ok_or_else(|| "janela Win32 ainda não está disponível".to_owned())?;
        let (id, url) = self
            .pending
            .clone()
            .ok_or_else(|| "WebEmbed sem ativação pendente".to_owned())?;

        self.ensure_com()?;
        let live = LiveWebView::new(parent, &url)?;
        self.current_id = Some(id);
        self.live = Some(live);
        Ok(())
    }

    fn capture_parent(&mut self, frame: &eframe::Frame) {
        if self.parent.is_some() {
            return;
        }
        let Ok(handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(window) = handle.as_raw() else {
            return;
        };
        self.parent = Some(HWND(window.hwnd.get() as *mut c_void));
    }
}

impl Default for WindowsWebEmbedBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEmbedBackend for WindowsWebEmbedBackend {
    fn create(&mut self, id: &str, url: &str) -> Result<(), String> {
        self.pending = Some((id.to_owned(), url.to_owned()));
        self.current_id = Some(id.to_owned());

        // Usually prepare_render() has already captured the HWND before the
        // user can click an embed. If activation happens earlier, creation is
        // deferred until present().
        if self.parent.is_some() {
            self.ensure_live()?;
        }
        Ok(())
    }

    fn present(&mut self, id: &str, viewport: &EmbedViewport) {
        if self.current_id.as_deref() != Some(id) {
            return;
        }
        if let Err(error) = self.ensure_live() {
            log::warn!("webembed(webview2): {error}");
            self.events.push(WebEmbedEvent::Failed { id: id.to_owned() });
            return;
        }
        if let Some(live) = &self.live
            && let Err(error) = live.set_bounds(viewport)
        {
            log::warn!("webembed(webview2): {error}");
            self.events.push(WebEmbedEvent::Failed { id: id.to_owned() });
        }
    }

    fn suspend(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id)
            && let Some(live) = &self.live
        {
            let _ = live.set_visible(false);
        }
    }

    fn resume(&mut self, _id: &str) {
        // Visibility is restored by the next present() with authoritative
        // geometry, avoiding one frame at stale coordinates.
    }

    fn destroy(&mut self, id: &str) {
        if self.current_id.as_deref() != Some(id) {
            return;
        }
        if let Some(live) = self.live.take() {
            live.close();
        }
        self.pending = None;
        self.current_id = None;
    }

    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        std::mem::take(&mut self.events)
    }

    fn prepare_render(&mut self, frame: &mut eframe::Frame) {
        self.capture_parent(frame);
    }

    fn is_playing(&self, id: &str) -> Option<bool> {
        if self.current_id.as_deref() != Some(id) {
            return None;
        }
        self.live.as_ref().and_then(LiveWebView::is_playing)
    }
}

impl Drop for WindowsWebEmbedBackend {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            live.close();
        }
        if self.com_initialized {
            unsafe { CoUninitialize() };
            self.com_initialized = false;
        }
    }
}
