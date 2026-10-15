//! Isolated passive channel views. No rich-link handlers are installed here.
use super::*;
use crate::channel_view::{ViewBackend, ViewEvent, refuses_framing, same_origin};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2_10, ICoreWebView2_13, ICoreWebView2Environment10, ICoreWebView2Settings3,
};
use webview2_com::{
    BasicAuthenticationRequestedEventHandler, WebResourceResponseReceivedEventHandler,
};
use windows::Win32::UI::{
    Input::KeyboardAndMouse::{GetFocus, SetFocus},
    WindowsAndMessaging::IsChild,
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
};

#[derive(Default)]
struct ViewEvents {
    queue: RefCell<Vec<ViewEvent>>,
    ctx: RefCell<Option<egui::Context>>,
}
impl ViewEvents {
    fn push(&self, event: ViewEvent) {
        self.queue.borrow_mut().push(event);
        if let Some(ctx) = &*self.ctx.borrow() {
            ctx.request_repaint();
        }
    }
}

static NEXT_PROFILE: AtomicU64 = AtomicU64::new(0);

type CleanupJobs = Rc<RefCell<Vec<std::thread::JoinHandle<()>>>>;

/// Retains a real process handle so directory deletion cannot race PID reuse.
struct Profile {
    directory: PathBuf,
    process: Option<usize>,
    cleanup_jobs: CleanupJobs,
}
impl Profile {
    fn new(cleanup_jobs: CleanupJobs) -> Result<Self, String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "papo-channel-view-{}-{nonce}-{}",
            std::process::id(),
            NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        Ok(Self {
            directory,
            process: None,
            cleanup_jobs,
        })
    }
    fn watch(&mut self, webview: &ICoreWebView2) -> windows::core::Result<()> {
        let mut pid = 0;
        unsafe {
            webview.BrowserProcessId(&mut pid)?;
            self.process = Some(OpenProcess(PROCESS_SYNCHRONIZE, false, pid)?.0 as usize);
        }
        Ok(())
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let path = self.directory.clone();
        let process = self.process.take();
        // No COM interfaces cross this thread. The directory is removed only
        // after the dedicated browser exits, never while it owns the profile.
        let job = std::thread::spawn(move || {
            if let Some(raw) = process {
                let handle = HANDLE(raw as *mut c_void);
                let exited = unsafe { WaitForSingleObject(handle, 5_000) } == WAIT_OBJECT_0;
                let _ = unsafe { CloseHandle(handle) };
                if !exited {
                    log::error!(
                        "channel-view: não foi possível aguardar o browser: {}",
                        path.display()
                    );
                    return;
                }
            }
            // Chromium utility processes can release file handles just after
            // the browser process exits. Retry those transient sharing errors.
            for _ in 0..25 {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => return,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            log::error!(
                "channel-view: perfil temporário não removido: {}",
                path.display()
            );
        });
        self.cleanup_jobs.borrow_mut().push(job);
    }
}

struct StrictLive {
    live: Rc<LiveWebView>,
    _environment: ICoreWebView2Environment,
    profile: Option<Profile>,
}
impl Drop for StrictLive {
    fn drop(&mut self) {
        self.live.close();
        // Release every COM owner before the cleanup worker waits for exit.
        // The environment field is released after Drop; the worker can wait.
        self.profile.take();
    }
}

pub struct WindowsChannelView {
    parent: Option<HWND>,
    live: Option<StrictLive>,
    events: Rc<ViewEvents>,
    initialized: bool,
    cleanup_jobs: CleanupJobs,
}
impl Default for WindowsChannelView {
    fn default() -> Self {
        Self::new()
    }
}
impl WindowsChannelView {
    pub fn new() -> Self {
        // Share only the geometry hook, never the rich-link browser/profile.
        eframe::egui_wgpu::set_post_present_hook(flush_presented_geometry);
        Self {
            parent: None,
            live: None,
            events: Default::default(),
            initialized: false,
            cleanup_jobs: Default::default(),
        }
    }
    fn build(&mut self, url: &str) -> Result<StrictLive, String> {
        let parent = self.parent.ok_or("janela Win32 indisponível")?;
        let mut profile = Profile::new(self.cleanup_jobs.clone())?;
        let directory = profile
            .directory
            .to_str()
            .ok_or("perfil temporário inválido")?;
        let options: ICoreWebView2EnvironmentOptions =
            CoreWebView2EnvironmentOptions::default().into();
        unsafe { options.SetAllowSingleSignOnUsingOSPrimaryAccount(false) }
            .map_err(|e| e.to_string())?;
        let directory = CoTaskMemPWSTR::from(directory);
        let (tx, rx) = mpsc::channel();
        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    windows::core::PCWSTR::null(),
                    *directory.as_ref().as_pcwstr(),
                    &options,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error, env| {
                error?;
                tx.send(env.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                    .map_err(|_| windows::core::Error::from(E_POINTER))?;
                Ok(())
            }),
        )
        .map_err(|e| e.to_string())?;
        let environment = rx
            .recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let env10: ICoreWebView2Environment10 = environment
            .cast()
            .map_err(|e| format!("InPrivate indisponível: {e}"))?;
        // WebView2 supports external user-data-folder overrides. Verify what
        // the runtime actually chose before allowing it to create a view.
        let mut actual_folder = PWSTR::null();
        unsafe { env10.UserDataFolder(&mut actual_folder) }.map_err(|e| e.to_string())?;
        let actual_folder = pwstr_string(actual_folder).ok_or("diretório WebView2 ausente")?;
        let actual_folder = std::fs::canonicalize(actual_folder).map_err(|e| e.to_string())?;
        let expected_folder =
            std::fs::canonicalize(&profile.directory).map_err(|e| e.to_string())?;
        if actual_folder != expected_folder {
            return Err("WebView2 substituiu o perfil isolado".into());
        }
        let host = create_clip_host(parent)?;
        let result = (|| -> windows::core::Result<_> {
            let options = unsafe { env10.CreateCoreWebView2ControllerOptions()? };
            unsafe {
                options.SetProfileName(w!("PassiveChannelView"))?;
                options.SetIsInPrivateModeEnabled(true)?;
            }
            let (tx, rx) = mpsc::channel();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |handler| unsafe {
                    env10
                        .CreateCoreWebView2ControllerWithOptions(host, &options, &handler)
                        .map_err(webview2_com::Error::WindowsError)
                }),
                Box::new(move |error, controller| {
                    error?;
                    tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                        .map_err(|_| windows::core::Error::from(E_POINTER))?;
                    Ok(())
                }),
            )
            .map_err(|_| windows::core::Error::from(E_POINTER))?;
            let controller = rx
                .recv()
                .map_err(|_| windows::core::Error::from(E_POINTER))??;
            let webview = unsafe { controller.CoreWebView2()? };
            let configured = (|| -> windows::core::Result<()> {
                profile.watch(&webview)?;
                unsafe {
                    controller.SetIsVisible(false)?;
                    let profile_view: ICoreWebView2_13 = webview.cast()?;
                    let mut private = BOOL(0);
                    profile_view
                        .Profile()?
                        .IsInPrivateModeEnabled(&mut private)?;
                    if !private.as_bool() {
                        return Err(windows::core::Error::from(
                            windows::Win32::Foundation::E_ACCESSDENIED,
                        ));
                    }
                    let geometry: ICoreWebView2Controller3 = controller.cast()?;
                    geometry.SetShouldDetectMonitorScaleChanges(false)?;
                    geometry.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)?;
                    let settings = webview.Settings()?;
                    settings.SetAreHostObjectsAllowed(false)?;
                    settings.SetIsWebMessageEnabled(false)?;
                    settings.SetAreDevToolsEnabled(false)?;
                    settings.SetAreDefaultContextMenusEnabled(false)?;
                    settings.SetAreDefaultScriptDialogsEnabled(false)?;
                    settings.SetIsStatusBarEnabled(false)?;
                    settings.SetIsBuiltInErrorPageEnabled(false)?;
                    let settings3: ICoreWebView2Settings3 = settings.cast()?;
                    settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
                    let settings4: ICoreWebView2Settings4 = settings.cast()?;
                    settings4.SetIsPasswordAutosaveEnabled(false)?;
                    settings4.SetIsGeneralAutofillEnabled(false)?;
                }
                install_handlers(&webview, &controller, url, &self.events)?;
                let uri = CoTaskMemPWSTR::from(url);
                // Ordinary Navigate: no Referer and no custom request headers.
                unsafe {
                    webview.Navigate(*uri.as_ref().as_pcwstr())?;
                }
                Ok(())
            })();
            if let Err(error) = configured {
                let _ = unsafe { controller.Close() };
                return Err(error);
            }
            Ok((controller, webview))
        })();
        match result {
            Ok((controller, webview)) => Ok(StrictLive {
                live: Rc::new(LiveWebView {
                    parent,
                    host,
                    controller,
                    webview,
                    fullscreen: Rc::new(Cell::new(false)),
                }),
                _environment: environment,
                profile: Some(profile),
            }),
            Err(error) => {
                let _ = unsafe { DestroyWindow(host) };
                Err(error.to_string())
            }
        }
    }
}
impl ViewBackend for WindowsChannelView {
    fn create(&mut self, url: &str) -> Result<(), String> {
        self.destroy();
        if !self.initialized {
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
                .map_err(|e| e.to_string())?;
            self.initialized = true;
        }
        self.live = Some(self.build(url)?);
        Ok(())
    }
    fn prepare(&mut self, frame: &mut eframe::Frame, ctx: &egui::Context) {
        self.events.ctx.replace(Some(ctx.clone()));
        self.cleanup_jobs
            .borrow_mut()
            .retain(|job| !job.is_finished());
        if let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::Win32(window) = handle.as_raw()
        {
            self.parent = Some(HWND(window.hwnd.get() as *mut c_void));
        }
    }
    fn present(&mut self, viewport: &EmbedViewport) {
        if let Some(live) = &self.live {
            live.live.resume();
            queue_geometry(&live.live, viewport);
        }
    }
    fn hide(&mut self) {
        if let Some(live) = &self.live {
            // The existing player also uses this geometry slot. Clear only a
            // pending presentation belonging to this channel view.
            PENDING_GEOMETRY.with(|pending| {
                let ours = pending.borrow().as_ref().is_some_and(|(weak, _)| {
                    weak.upgrade()
                        .is_some_and(|queued| Rc::ptr_eq(&queued, &live.live))
                });
                if ours {
                    pending.take();
                }
            });
            unsafe {
                let focus = GetFocus();
                if focus == live.live.host || IsChild(live.live.host, focus).as_bool() {
                    let _ = SetFocus(Some(live.live.parent));
                }
            }
            // Keep the initial load running while hidden. Suspending here
            // could prevent a page from ever reaching its readiness checks.
            if live.live.set_visible(false).is_err() {
                self.events.push(ViewEvent::Failed);
            }
        }
    }
    fn destroy(&mut self) {
        self.hide();
        self.live = None;
        self.events.queue.borrow_mut().clear();
    }
    fn poll(&mut self) -> Vec<ViewEvent> {
        std::mem::take(&mut *self.events.queue.borrow_mut())
    }
}
impl Drop for WindowsChannelView {
    fn drop(&mut self) {
        self.destroy();
        // on_exit closes the view, and Drop waits for its bounded cleanup jobs
        // before the application process can terminate those worker threads.
        for job in self.cleanup_jobs.borrow_mut().drain(..) {
            if job.join().is_err() {
                log::error!("channel-view: thread de limpeza falhou");
            }
        }
        if self.initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn install_handlers(
    webview: &ICoreWebView2,
    controller: &ICoreWebView2Controller,
    origin: &str,
    events: &Rc<ViewEvents>,
) -> windows::core::Result<()> {
    let pending = Rc::new(RefCell::new(origin.to_owned()));
    let checked = Rc::new(Cell::new(false));
    let locked = origin.to_owned();
    let navigation_events = events.clone();
    let navigation_uri = pending.clone();
    let navigation_checked = checked.clone();
    let navigation_controller = controller.clone();
    let navigation = NavigationStartingEventHandler::create(Box::new(move |_sender, args| {
        let result = (|| -> windows::core::Result<()> {
            let args = args.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
            let mut raw = PWSTR::null();
            unsafe {
                navigation_controller.SetIsVisible(false)?;
                args.Uri(&mut raw)?;
            }
            let uri = pwstr_string(raw).ok_or_else(|| windows::core::Error::from(E_POINTER))?;
            if !same_origin(&locked, &uri) {
                unsafe {
                    args.SetCancel(true)?;
                }
                navigation_events.push(ViewEvent::Failed);
            } else {
                *navigation_uri.borrow_mut() = uri;
                navigation_checked.set(false);
                navigation_events.push(ViewEvent::Loading);
            }
            Ok(())
        })();
        if result.is_err() {
            navigation_events.push(ViewEvent::Failed);
        }
        result
    }));
    let response_events = events.clone();
    let response_uri = pending.clone();
    let response_checked = checked.clone();
    let response =
        WebResourceResponseReceivedEventHandler::create(Box::new(move |sender, args| {
            let result = (|| -> windows::core::Result<()> {
                let args = args.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
                let request = unsafe { args.Request()? };
                let mut raw = PWSTR::null();
                unsafe {
                    request.Uri(&mut raw)?;
                }
                let uri = pwstr_string(raw).ok_or_else(|| windows::core::Error::from(E_POINTER))?;
                let headers = unsafe { request.Headers()? };
                let mut raw = PWSTR::null();
                // Chromium's browser-owned Fetch Metadata distinguishes documents
                // from iframes/fetches, including when their URLs are identical.
                let destination = unsafe { headers.GetHeader(w!("Sec-Fetch-Dest"), &mut raw) }
                    .ok()
                    .and_then(|()| pwstr_string(raw))
                    .unwrap_or_default();
                if destination != "document" {
                    return Ok(());
                }
                if uri != *response_uri.borrow() {
                    return Ok(());
                }
                let response = unsafe { args.Response()? };
                let headers = unsafe { response.Headers()? };
                let xfo = response_header(&headers, w!("X-Frame-Options"))?;
                let csp = response_header(&headers, w!("Content-Security-Policy"))?;
                let mut status = 0;
                unsafe {
                    response.StatusCode(&mut status)?;
                }
                if refuses_framing(&xfo, &csp) || status >= 400 {
                    if let Some(sender) = sender {
                        unsafe {
                            sender.Stop()?;
                        }
                    }
                    response_events.push(ViewEvent::Failed);
                } else {
                    response_checked.set(true);
                }
                Ok(())
            })();
            if result.is_err() {
                response_events.push(ViewEvent::Failed);
            }
            result
        }));
    let complete_events = events.clone();
    let completed = NavigationCompletedEventHandler::create(Box::new(move |_sender, args| {
        let mut success = BOOL(0);
        let result = args
            .ok_or_else(|| windows::core::Error::from(E_POINTER))
            .and_then(|args| unsafe { args.IsSuccess(&mut success) });
        complete_events.push(if result.is_ok() && success.as_bool() && checked.get() {
            ViewEvent::Ready
        } else {
            ViewEvent::Failed
        });
        result
    }));
    let popup = NewWindowRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                args.SetHandled(true)?;
            }
        }
        Ok(())
    }));
    let permission = PermissionRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
            }
        }
        Ok(())
    }));
    let download = DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                args.SetCancel(true)?;
            }
        }
        Ok(())
    }));
    let auth = BasicAuthenticationRequestedEventHandler::create(Box::new(move |_sender, args| {
        if let Some(args) = args {
            unsafe {
                args.SetCancel(true)?;
            }
        }
        Ok(())
    }));
    let failed_events = events.clone();
    let failure = ProcessFailedEventHandler::create(Box::new(move |_sender, _args| {
        failed_events.push(ViewEvent::Failed);
        Ok(())
    }));
    let request_events = events.clone();
    let request_controller = controller.clone();
    let request = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
        let result = (|| -> windows::core::Result<()> {
            let args = args.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
            let headers = unsafe { args.Request()?.Headers()? };
            // Do not add app identity or any custom header. Also strip the
            // engine-generated Referer on redirects and ordinary page requests.
            unsafe {
                headers.RemoveHeader(w!("Referer"))?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = unsafe { request_controller.SetIsVisible(false) };
            if let Some(sender) = sender {
                let _ = unsafe { sender.Stop() };
            }
            request_events.push(ViewEvent::Failed);
        }
        result
    }));
    let mut token = 0;
    unsafe {
        // Every interface is mandatory, never best-effort.
        let response_view: ICoreWebView2_2 = webview.cast()?;
        let downloads: ICoreWebView2_4 = webview.cast()?;
        let authentication: ICoreWebView2_10 = webview.cast()?;
        webview.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        webview.add_WebResourceRequested(&request, &mut token)?;
        webview.add_NavigationStarting(&navigation, &mut token)?;
        response_view.add_WebResourceResponseReceived(&response, &mut token)?;
        webview.add_NavigationCompleted(&completed, &mut token)?;
        webview.add_NewWindowRequested(&popup, &mut token)?;
        webview.add_PermissionRequested(&permission, &mut token)?;
        downloads.add_DownloadStarting(&download, &mut token)?;
        authentication.add_BasicAuthenticationRequested(&auth, &mut token)?;
        webview.add_ProcessFailed(&failure, &mut token)?;
    }
    Ok(())
}

fn response_header(
    headers: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2HttpResponseHeaders,
    name: windows::core::PCWSTR,
) -> windows::core::Result<String> {
    let mut present = BOOL(0);
    unsafe {
        headers.Contains(name, &mut present)?;
    }
    if !present.as_bool() {
        return Ok(String::new());
    }
    let mut value = PWSTR::null();
    unsafe {
        headers.GetHeader(name, &mut value)?;
    }
    pwstr_string(value).ok_or_else(|| windows::core::Error::from(E_POINTER))
}
