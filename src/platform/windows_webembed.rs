//! Native Windows WebEmbed backend using Edge WebView2 directly.
//!
//! The shared WebEmbed manager owns lifetime/policy. WebView2 owns Chromium,
//! native rendering and native input. A tiny child HWND acts as a clipping
//! container so partially-scrolled embeds are cropped without resizing the
//! browser viewport (the same geometry model as Android's native wrapper).

#![cfg(target_os = "windows")]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::mpsc;

use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::{BOOL, E_POINTER, HWND, RECT},
        System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize, IStream,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetClientRect, SW_HIDE, SW_SHOW,
            SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, WS_CHILD, WS_CLIPCHILDREN,
            WS_CLIPSIBLINGS,
        },
    },
    core::{Interface as _, PWSTR, w},
};
use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    ContainsFullScreenElementChangedEventHandler, DownloadStartingEventHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, ProcessFailedEventHandler,
    TrySuspendCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_STATE_DENY, CreateCoreWebView2Environment,
        ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Environment,
        ICoreWebView2Environment2, ICoreWebView2_2, ICoreWebView2_3,
        ICoreWebView2_4, ICoreWebView2_8,
    },
};

use crate::webembed::{EmbedViewport, WebEmbedBackend, WebEmbedEvent};

const APP_REFERER: &str = "https://io.github.chriexpe.papo/";

struct EventSink {
    events: RefCell<Vec<WebEmbedEvent>>,
    egui: RefCell<Option<egui::Context>>,
}

impl EventSink {
    fn new() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
            egui: RefCell::new(None),
        }
    }

    fn set_context(&self, ctx: &egui::Context) {
        self.egui.replace(Some(ctx.clone()));
    }

    fn wake(&self) {
        if let Some(ctx) = self.egui.borrow().as_ref() {
            ctx.request_repaint();
        }
    }

    fn push(&self, event: WebEmbedEvent) {
        self.events.borrow_mut().push(event);
        self.wake();
    }

    fn drain(&self) -> Vec<WebEmbedEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

type EventQueue = Rc<EventSink>;

struct LiveWebView {
    parent: HWND,
    host: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    fullscreen: Rc<Cell<bool>>,
}

impl LiveWebView {
    fn new(
        parent: HWND,
        id: &str,
        url: &str,
        events: &EventQueue,
    ) -> Result<Self, String> {
        let host = create_clip_host(parent)?;
        let environment = create_environment()?;

        let controller = match create_controller(&environment, host) {
            Ok(controller) => controller,
            Err(error) => {
                // SAFETY: host belongs exclusively to this failed construction.
                let _ = unsafe { DestroyWindow(host) };
                return Err(error);
            }
        };
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
            settings
                .SetAreHostObjectsAllowed(false)
                .map_err(|error| format!("desativar host objects WebView2: {error}"))?;
            settings
                .SetIsWebMessageEnabled(false)
                .map_err(|error| format!("desativar web messages WebView2: {error}"))?;
            settings
                .SetAreDefaultScriptDialogsEnabled(false)
                .map_err(|error| format!("desativar dialogs WebView2: {error}"))?;
            settings
                .SetIsStatusBarEnabled(false)
                .map_err(|error| format!("desativar status bar WebView2: {error}"))?;
            controller
                .SetIsVisible(false)
                .map_err(|error| format!("ocultar WebView2 inicial: {error}"))?;
        }

        let fullscreen = Rc::new(Cell::new(false));
        install_security_and_navigation_handlers(&webview, id, events, &fullscreen)?;
        navigate_with_referer(&environment, &webview, url)?;

        Ok(Self {
            parent,
            host,
            controller,
            webview,
            fullscreen,
        })
    }

    fn set_bounds(&self, viewport: &EmbedViewport) -> Result<(), String> {
        if self.fullscreen.get() {
            let mut client = RECT::default();
            unsafe {
                GetClientRect(self.parent, &mut client)
                    .map_err(|error| format!("medir janela para fullscreen WebView2: {error}"))?;
                let width = (client.right - client.left).max(1);
                let height = (client.bottom - client.top).max(1);
                SetWindowPos(
                    self.host,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .map_err(|error| format!("posicionar fullscreen WebView2: {error}"))?;
                self.controller
                    .SetBounds(RECT {
                        left: 0,
                        top: 0,
                        right: width,
                        bottom: height,
                    })
                    .map_err(|error| format!("redimensionar fullscreen WebView2: {error}"))?;
                self.controller
                    .NotifyParentWindowPositionChanged()
                    .map_err(|error| format!("notificar fullscreen WebView2: {error}"))?;
                let _ = ShowWindow(self.host, SW_SHOW);
                self.controller
                    .SetIsVisible(true)
                    .map_err(|error| format!("mostrar fullscreen WebView2: {error}"))?;
            }
            return Ok(());
        }

        let clipped = viewport.rect.intersect(viewport.clip_rect);
        if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
            return self.set_visible(false);
        }

        let scale = viewport.pixels_per_point.max(0.1);
        let full_left = (viewport.rect.min.x * scale).round() as i32;
        let full_top = (viewport.rect.min.y * scale).round() as i32;
        let full_width = (viewport.rect.width() * scale).round().max(1.0) as i32;
        let full_height = (viewport.rect.height() * scale).round().max(1.0) as i32;

        let clip_left = (clipped.min.x * scale).round() as i32;
        let clip_top = (clipped.min.y * scale).round() as i32;
        let clip_width = (clipped.width() * scale).round().max(1.0) as i32;
        let clip_height = (clipped.height() * scale).round().max(1.0) as i32;

        // The clipping HWND occupies only the visible intersection. The actual
        // WebView keeps its full logical size and is translated inside that
        // child window, so scrolling never causes YouTube/other providers to
        // relayout merely because part of the card is offscreen.
        unsafe {
            SetWindowPos(
                self.host,
                None,
                clip_left,
                clip_top,
                clip_width,
                clip_height,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .map_err(|error| format!("posicionar host WebView2: {error}"))?;

            self.controller
                .SetBounds(RECT {
                    left: full_left - clip_left,
                    top: full_top - clip_top,
                    right: full_left - clip_left + full_width,
                    bottom: full_top - clip_top + full_height,
                })
                .map_err(|error| format!("posicionar WebView2: {error}"))?;
            self.controller
                .NotifyParentWindowPositionChanged()
                .map_err(|error| format!("notificar posição WebView2: {error}"))?;
            let _ = ShowWindow(self.host, SW_SHOW);
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
                .map_err(|error| format!("visibilidade WebView2: {error}"))?;
            let _ = ShowWindow(self.host, if visible { SW_SHOW } else { SW_HIDE });
        }
        Ok(())
    }

    fn suspend(&self) {
        if self.set_visible(false).is_err() {
            return;
        }
        let Ok(webview) = self.webview.cast::<ICoreWebView2_3>() else {
            return;
        };
        let handler = TrySuspendCompletedHandler::create(Box::new(
            |_error_code, _successful| Ok(()),
        ));
        // SAFETY: controller visibility was set false above, which WebView2
        // requires before TrySuspend. This is best-effort by design.
        if let Err(error) = unsafe { webview.TrySuspend(&handler) } {
            log::debug!("webembed(webview2): TrySuspend ignorado: {error}");
        }
    }

    fn resume(&self) {
        if let Ok(webview) = self.webview.cast::<ICoreWebView2_3>() {
            // SAFETY: live WebView2 on its owning STA thread.
            if let Err(error) = unsafe { webview.Resume() } {
                log::debug!("webembed(webview2): Resume ignorado: {error}");
            }
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
        // SAFETY: host is the clipping child HWND created for this WebView.
        let _ = unsafe { DestroyWindow(self.host) };
    }
}

fn create_clip_host(parent: HWND) -> Result<HWND, String> {
    // Use the system STATIC class solely as a clipping container. It has no
    // Papo drawing or event policy of its own.
    unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!(""),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            None,
            None,
        )
        .map_err(|error| format!("criar host filho do WebView2: {error}"))
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

fn navigate_with_referer(
    environment: &ICoreWebView2Environment,
    webview: &ICoreWebView2,
    url: &str,
) -> Result<(), String> {
    let environment: ICoreWebView2Environment2 = environment
        .cast()
        .map_err(|error| format!("WebView2 Environment2 indisponível: {error}"))?;
    let webview: ICoreWebView2_2 = webview
        .cast()
        .map_err(|error| format!("WebView2 v2 indisponível: {error}"))?;

    let uri = CoTaskMemPWSTR::from(url);
    let method = CoTaskMemPWSTR::from("GET");
    let headers = CoTaskMemPWSTR::from(format!("Referer: {APP_REFERER}\r\n").as_str());

    let request = unsafe {
        environment.CreateWebResourceRequest(
            *uri.as_ref().as_pcwstr(),
            *method.as_ref().as_pcwstr(),
            None::<&IStream>,
            *headers.as_ref().as_pcwstr(),
        )
    }
    .map_err(|error| format!("criar request WebView2: {error}"))?;

    unsafe { webview.NavigateWithWebResourceRequest(&request) }
        .map_err(|error| format!("navegar WebView2: {error}"))
}

fn install_security_and_navigation_handlers(
    webview: &ICoreWebView2,
    id: &str,
    events: &EventQueue,
    fullscreen: &Rc<Cell<bool>>,
) -> Result<(), String> {
    let initial_navigation_done = Rc::new(Cell::new(false));

    let navigation_events = Rc::clone(events);
    let navigation_id = id.to_owned();
    let navigation_done = Rc::clone(&initial_navigation_done);
    let navigation = NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else {
            return Ok(());
        };

        let mut uri = PWSTR::null();
        let mut user_initiated = BOOL(0);
        unsafe {
            args.Uri(&mut uri)?;
            args.IsUserInitiated(&mut user_initiated)?;
        }
        let Some(url) = pwstr_string(uri) else {
            return Ok(());
        };

        // WebView2 reports API-initiated NavigateWithWebResourceRequest as user
        // initiated too, so do not externalize until the initial document has
        // completed. Afterwards, a user gesture that wants to replace the
        // top-level embed becomes an ordinary external link.
        if navigation_done.get() && user_initiated.as_bool() {
            unsafe { args.SetCancel(true)? };
            if papo_core::preview::safe_remote_url(&url) {
                navigation_events.push(WebEmbedEvent::OpenExternal {
                    id: navigation_id.clone(),
                    url,
                });
            }
        }
        Ok(())
    }));

    let complete_done = Rc::clone(&initial_navigation_done);
    let completed = NavigationCompletedEventHandler::create(Box::new(move |_sender, _args| {
        complete_done.set(true);
        Ok(())
    }));

    let window_events = Rc::clone(events);
    let window_id = id.to_owned();
    let new_window = NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else {
            return Ok(());
        };

        // Never let third-party content create native popup windows owned by
        // Papo. User-initiated targets are handed to the normal external-link
        // trust/open flow; scripted popups are silently blocked.
        let mut uri = PWSTR::null();
        let mut user_initiated = BOOL(0);
        unsafe {
            args.Uri(&mut uri)?;
            args.IsUserInitiated(&mut user_initiated)?;
            args.SetHandled(true)?;
        }
        if user_initiated.as_bool()
            && let Some(url) = pwstr_string(uri)
            && papo_core::preview::safe_remote_url(&url)
        {
            window_events.push(WebEmbedEvent::OpenExternal {
                id: window_id.clone(),
                url,
            });
        }
        Ok(())
    }));

    let permission = PermissionRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            // Embedded third-party content never receives camera, microphone,
            // geolocation, clipboard, notification or other browser grants.
            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)? };
        }
        Ok(())
    }));

    let download = DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            // WebEmbed is playback/preview surface, not a download surface.
            // Attachments/downloads go through Papo's explicit file flow.
            unsafe { args.SetCancel(true)? };
        }
        Ok(())
    }));

    let fullscreen_state = Rc::clone(fullscreen);
    let fullscreen_events = Rc::clone(events);
    let fullscreen_changed =
        ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _args| {
            let Some(sender) = sender else {
                return Ok(());
            };
            let mut contains = BOOL(0);
            unsafe { sender.ContainsFullScreenElement(&mut contains)? };
            fullscreen_state.set(contains.as_bool());
            fullscreen_events.wake();
            Ok(())
        }));

    let failure_events = Rc::clone(events);
    let failure_id = id.to_owned();
    let process_failed = ProcessFailedEventHandler::create(Box::new(move |_sender, _args| {
        failure_events.push(WebEmbedEvent::Failed {
            id: failure_id.clone(),
        });
        Ok(())
    }));

    let mut token = 0;
    unsafe {
        webview
            .add_NavigationStarting(&navigation, &mut token)
            .map_err(|error| format!("NavigationStarting WebView2: {error}"))?;
        webview
            .add_NavigationCompleted(&completed, &mut token)
            .map_err(|error| format!("NavigationCompleted WebView2: {error}"))?;
        webview
            .add_NewWindowRequested(&new_window, &mut token)
            .map_err(|error| format!("NewWindowRequested WebView2: {error}"))?;
        webview
            .add_PermissionRequested(&permission, &mut token)
            .map_err(|error| format!("PermissionRequested WebView2: {error}"))?;
        let webview4: ICoreWebView2_4 = webview
            .cast()
            .map_err(|error| format!("WebView2 v4 indisponível: {error}"))?;
        webview4
            .add_DownloadStarting(&download, &mut token)
            .map_err(|error| format!("DownloadStarting WebView2: {error}"))?;
        webview
            .add_ContainsFullScreenElementChanged(&fullscreen_changed, &mut token)
            .map_err(|error| format!("Fullscreen WebView2: {error}"))?;
        webview
            .add_ProcessFailed(&process_failed, &mut token)
            .map_err(|error| format!("ProcessFailed WebView2: {error}"))?;
    }
    Ok(())
}

fn pwstr_string(value: PWSTR) -> Option<String> {
    if value.0.is_null() {
        return None;
    }
    Some(CoTaskMemPWSTR::from(value).to_string())
}

pub struct WindowsWebEmbedBackend {
    parent: Option<HWND>,
    pending: Option<(String, String)>,
    live: Option<LiveWebView>,
    current_id: Option<String>,
    events: EventQueue,
    com_initialized: bool,
}

impl WindowsWebEmbedBackend {
    pub fn new() -> Self {
        Self {
            parent: None,
            pending: None,
            live: None,
            current_id: None,
            events: Rc::new(EventSink::new()),
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
        let live = LiveWebView::new(parent, &id, &url, &self.events)?;
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

        // Normally prepare_render() has captured the HWND long before a click.
        // If activation occurs earlier, controller creation is deferred until
        // the first authoritative present().
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
            self.events
                .push(WebEmbedEvent::Failed { id: id.to_owned() });
            return;
        }
        if let Some(live) = &self.live
            && let Err(error) = live.set_bounds(viewport)
        {
            log::warn!("webembed(webview2): {error}");
            self.events
                .push(WebEmbedEvent::Failed { id: id.to_owned() });
        }
    }

    fn suspend(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id)
            && let Some(live) = &self.live
        {
            live.suspend();
        }
    }

    fn resume(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id)
            && let Some(live) = &self.live
        {
            live.resume();
        }
        // The next present() restores visibility at authoritative geometry,
        // never for a frame at stale coordinates.
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
        self.events.drain()
    }

    fn set_context(&mut self, ctx: &egui::Context) {
        self.events.set_context(ctx);
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
