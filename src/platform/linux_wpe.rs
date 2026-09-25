//! Toolkit-free WPE WebKit bridge used by Linux WebEmbed.
//!
//! This is intentionally a narrow consumer of the ABI shipped by
//! \`waterui-browser-wpe\` (ABI v3, MIT/Apache-2.0). Papo does not pull in
//! WaterUI/wgpu and does not link GTK/WebKit at build time.
//!
//! Runtime layout:
//!   <root>/lib/libwaterui_wpe.so
//! with the rest of the pinned WPE WebKit runtime staged beside it.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_uint, c_void};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;

const ABI_VERSION: u32 = 3;
pub const RUNTIME_ENV: &str = "PAPO_WPE_RUNTIME";

#[repr(C)]
struct WaterWpeRuntime {
    _private: [u8; 0],
}

#[repr(C)]
struct WaterWpePage {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawBytes {
    data: *const u8,
    len: usize,
    user_data: *mut c_void,
    destroy: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawFrame {
    token: *mut c_void,
    width: c_uint,
    height: c_uint,
    format: c_uint,
    modifier: u64,
    n_planes: c_uint,
    fds: [c_int; 4],
    offsets: [c_uint; 4],
    strides: [c_uint; 4],
    rendering_fence_fd: c_int,
}

type DestroyNotify = unsafe extern "C" fn(*mut c_void);
type EventCallback =
    unsafe extern "C" fn(*mut c_void, c_uint, *const c_char, *const c_char, c_double);
type FrameCallback = unsafe extern "C" fn(*mut c_void, *const RawFrame);
type MessageCallback =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> RawBytes;

struct Api {
    abi_version: unsafe extern "C" fn() -> c_uint,
    runtime_new: unsafe extern "C" fn(*mut *mut c_char) -> *mut WaterWpeRuntime,
    runtime_free: unsafe extern "C" fn(*mut WaterWpeRuntime),
    runtime_iteration: unsafe extern "C" fn(*mut WaterWpeRuntime) -> bool,
    string_free: unsafe extern "C" fn(*mut c_char),
    page_new: unsafe extern "C" fn(
        *mut WaterWpeRuntime,
        EventCallback,
        FrameCallback,
        MessageCallback,
        *mut c_void,
        DestroyNotify,
        *mut *mut c_char,
    ) -> *mut WaterWpePage,
    page_free: unsafe extern "C" fn(*mut WaterWpePage),
    page_load_uri: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    page_stop: unsafe extern "C" fn(*mut WaterWpePage),
    page_resize: unsafe extern "C" fn(*mut WaterWpePage, c_uint, c_uint, c_double),
    page_set_focus: unsafe extern "C" fn(*mut WaterWpePage, bool),
    page_pointer_button:
        unsafe extern "C" fn(*mut WaterWpePage, bool, c_uint, c_double, c_double, c_uint, c_uint),
    page_pointer_move: unsafe extern "C" fn(
        *mut WaterWpePage,
        c_double,
        c_double,
        c_double,
        c_double,
        c_uint,
        c_uint,
    ),
    page_scroll: unsafe extern "C" fn(
        *mut WaterWpePage,
        c_double,
        c_double,
        c_double,
        c_double,
        bool,
        bool,
        c_uint,
        c_uint,
    ),
    frame_presented: unsafe extern "C" fn(*mut c_void),
    frame_release: unsafe extern "C" fn(*mut c_void, c_int),
}

impl Api {
    unsafe fn load(library: &libloading::Library) -> Result<Self, String> {
        unsafe fn symbol<T: Copy>(
            library: &libloading::Library,
            name: &[u8],
        ) -> Result<T, String> {
            // SAFETY: signatures below are the pinned ABI-v3 contract and the
            // Library is retained for as long as these pointers exist.
            unsafe { library.get::<T>(name) }
                .map(|value| *value)
                .map_err(|error| format!("símbolo WPE ausente: {error}"))
        }

        // SAFETY: see module/ABI contract above.
        unsafe {
            Ok(Self {
                abi_version: symbol(library, b"water_wpe_abi_version\0")?,
                runtime_new: symbol(library, b"water_wpe_runtime_new\0")?,
                runtime_free: symbol(library, b"water_wpe_runtime_free\0")?,
                runtime_iteration: symbol(library, b"water_wpe_runtime_iteration\0")?,
                string_free: symbol(library, b"water_wpe_string_free\0")?,
                page_new: symbol(library, b"water_wpe_page_new\0")?,
                page_free: symbol(library, b"water_wpe_page_free\0")?,
                page_load_uri: symbol(library, b"water_wpe_page_load_uri\0")?,
                page_stop: symbol(library, b"water_wpe_page_stop\0")?,
                page_resize: symbol(library, b"water_wpe_page_resize\0")?,
                page_set_focus: symbol(library, b"water_wpe_page_set_focus\0")?,
                page_pointer_button: symbol(library, b"water_wpe_page_pointer_button\0")?,
                page_pointer_move: symbol(library, b"water_wpe_page_pointer_move\0")?,
                page_scroll: symbol(library, b"water_wpe_page_scroll\0")?,
                frame_presented: symbol(library, b"water_wpe_frame_presented\0")?,
                frame_release: symbol(library, b"water_wpe_frame_release\0")?,
            })
        }
    }
}

struct RuntimeApi {
    api: Api,
    _library: libloading::Library,
}

// SAFETY: the immutable symbol table points into the retained library. The
// bridge marshals frame completion to its own GMainContext.
unsafe impl Send for RuntimeApi {}
// SAFETY: same reasoning as Send.
unsafe impl Sync for RuntimeApi {}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    root: PathBuf,
}

impl RuntimePaths {
    pub fn discover() -> Result<Self, String> {
        if let Some(root) = std::env::var_os(RUNTIME_ENV) {
            return Ok(Self {
                root: PathBuf::from(root),
            });
        }
        let executable = std::env::current_exe()
            .map_err(|error| format!("não foi possível localizar o executável: {error}"))?;
        let directory = executable
            .parent()
            .ok_or_else(|| "executável sem diretório pai".to_owned())?;
        Ok(Self {
            root: directory.join("waterui-browser/wpe"),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn bridge(&self) -> PathBuf {
        self.root.join("lib/libwaterui_wpe.so")
    }

    fn validate(&self) -> Result<(), String> {
        let bridge = self.bridge();
        if bridge.is_file() {
            Ok(())
        } else {
            Err(format!(
                "ponte WPE WebKit não encontrada em {} (rode scripts/wpe-runtime.sh ou defina {RUNTIME_ENV})",
                self.root.display()
            ))
        }
    }
}

struct RuntimeInner {
    api: Arc<RuntimeApi>,
    raw: NonNull<WaterWpeRuntime>,
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        // SAFETY: this object owns the runtime exactly once.
        unsafe { (self.api.api.runtime_free)(self.raw.as_ptr()) };
    }
}

#[derive(Clone)]
pub struct Runtime {
    inner: Rc<RuntimeInner>,
}

impl Runtime {
    pub fn open(paths: &RuntimePaths) -> Result<Self, String> {
        paths.validate()?;
        let bridge = paths.bridge();
        // SAFETY: explicit, user-/package-selected bridge path.
        let library = unsafe { libloading::Library::new(&bridge) }
            .map_err(|error| format!("falha ao carregar {}: {error}", bridge.display()))?;
        // SAFETY: the ABI version is checked before any runtime object is used.
        let api = unsafe { Api::load(&library)? };
        // SAFETY: pure version query.
        let version = unsafe { (api.abi_version)() };
        if version != ABI_VERSION {
            return Err(format!(
                "ABI WPE incompatível: runtime={version}, esperado={ABI_VERSION}"
            ));
        }

        let api = Arc::new(RuntimeApi {
            api,
            _library: library,
        });
        let mut error = std::ptr::null_mut::<c_char>();
        // SAFETY: constructor from the validated ABI.
        let raw = unsafe { (api.api.runtime_new)(&raw mut error) };
        let raw = NonNull::new(raw).ok_or_else(|| take_error(&api, error))?;
        if !error.is_null() {
            return Err("runtime WPE retornou sucesso e erro ao mesmo tempo".to_owned());
        }
        Ok(Self {
            inner: Rc::new(RuntimeInner { api, raw }),
        })
    }

    pub fn pump(&self) {
        // SAFETY: raw is the live runtime owned by inner.
        while unsafe { (self.inner.api.api.runtime_iteration)(self.inner.raw.as_ptr()) } {}
    }
}

#[derive(Debug, Clone)]
pub enum PageEvent {
    NavigationStarted(String),
    Loaded,
    LoadFailed(String),
}

struct PageState {
    api: Arc<RuntimeApi>,
    events: RefCell<Vec<PageEvent>>,
    frame: RefCell<Option<Frame>>,
    frame_waker: RefCell<Option<Rc<dyn Fn()>>>,
    size: Cell<(u32, u32, f64)>,
}

struct PageInner {
    runtime: Runtime,
    raw: NonNull<WaterWpePage>,
    state: Rc<PageState>,
}

impl Drop for PageInner {
    fn drop(&mut self) {
        // SAFETY: one live page, freed exactly once.
        unsafe { (self.runtime.inner.api.api.page_free)(self.raw.as_ptr()) };
    }
}

#[derive(Clone)]
pub struct Page {
    inner: Rc<PageInner>,
}

impl Page {
    pub fn new(runtime: Runtime) -> Result<Self, String> {
        let state = Rc::new(PageState {
            api: Arc::clone(&runtime.inner.api),
            events: RefCell::new(Vec::new()),
            frame: RefCell::new(None),
            frame_waker: RefCell::new(None),
            size: Cell::new((1, 1, 1.0)),
        });
        let context = Box::into_raw(Box::new(Rc::clone(&state)));
        let mut error = std::ptr::null_mut::<c_char>();
        // SAFETY: callbacks and context stay valid until destroy_state is called
        // by page_free.
        let raw = unsafe {
            (runtime.inner.api.api.page_new)(
                runtime.inner.raw.as_ptr(),
                event_callback,
                frame_callback,
                message_callback,
                context.cast(),
                destroy_state,
                &raw mut error,
            )
        };
        let Some(raw) = NonNull::new(raw) else {
            // page_new owns/destroys context on failure according to ABI.
            return Err(take_error(&runtime.inner.api, error));
        };
        if !error.is_null() {
            return Err("página WPE retornou sucesso e erro ao mesmo tempo".to_owned());
        }
        Ok(Self {
            inner: Rc::new(PageInner {
                runtime,
                raw,
                state,
            }),
        })
    }

    pub fn load(&self, url: &str) -> Result<(), String> {
        let url = c_string(url, "URL WPE")?;
        // SAFETY: live page and NUL-terminated URL.
        unsafe { (self.inner.state.api.api.page_load_uri)(self.inner.raw.as_ptr(), url.as_ptr()) };
        Ok(())
    }

    pub fn stop(&self) {
        // SAFETY: live page.
        unsafe { (self.inner.state.api.api.page_stop)(self.inner.raw.as_ptr()) };
    }

    pub fn resize(&self, width: u32, height: u32, scale: f64) {
        let width = width.max(1);
        let height = height.max(1);
        if self.inner.state.size.replace((width, height, scale)) == (width, height, scale) {
            return;
        }
        // SAFETY: live page, validated non-zero dimensions.
        unsafe {
            (self.inner.state.api.api.page_resize)(
                self.inner.raw.as_ptr(),
                width,
                height,
                scale.max(0.1),
            )
        };
    }

    pub fn set_focus(&self, focused: bool) {
        // SAFETY: live page.
        unsafe { (self.inner.state.api.api.page_set_focus)(self.inner.raw.as_ptr(), focused) };
    }

    pub fn pointer_button(&self, pressed: bool, x: f64, y: f64, time_ms: u32) {
        // WPEPlatform button 1 is primary/left.
        // SAFETY: live page.
        unsafe {
            (self.inner.state.api.api.page_pointer_button)(
                self.inner.raw.as_ptr(),
                pressed,
                1,
                x,
                y,
                0,
                time_ms,
            )
        };
    }

    pub fn pointer_move(&self, x: f64, y: f64, dx: f64, dy: f64, time_ms: u32) {
        // SAFETY: live page.
        unsafe {
            (self.inner.state.api.api.page_pointer_move)(
                self.inner.raw.as_ptr(),
                x,
                y,
                dx,
                dy,
                0,
                time_ms,
            )
        };
    }

    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64, time_ms: u32) {
        // SAFETY: live page.
        unsafe {
            (self.inner.state.api.api.page_scroll)(
                self.inner.raw.as_ptr(),
                x,
                y,
                dx,
                dy,
                true,
                false,
                0,
                time_ms,
            )
        };
    }

    pub fn set_frame_waker(&self, waker: impl Fn() + 'static) {
        self.inner.state.frame_waker.replace(Some(Rc::new(waker)));
    }

    pub fn take_frame(&self) -> Option<Frame> {
        self.inner.state.frame.borrow_mut().take()
    }

    pub fn take_events(&self) -> Vec<PageEvent> {
        std::mem::take(&mut *self.inner.state.events.borrow_mut())
    }

    pub fn pump(&self) {
        self.inner.runtime.pump();
    }
}

pub struct Frame {
    api: Arc<RuntimeApi>,
    token: *mut c_void,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub modifier: u64,
    pub plane: OwnedFd,
    pub offset: u32,
    pub stride: u32,
    pub rendering_fence: Option<OwnedFd>,
    completed: bool,
}

impl Frame {
    /// Marks the buffer imported/copied and returns it to WPE. The initial
    /// renderer will call this only after its GL work has finished.
    pub fn release(mut self) {
        // SAFETY: token is the lease handed to this frame callback exactly once.
        unsafe {
            (self.api.api.frame_presented)(self.token);
            (self.api.api.frame_release)(self.token, -1);
        }
        self.completed = true;
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // An abandoned frame still has to return to WPE's buffer pool.
        // SAFETY: same lease contract as release().
        unsafe {
            (self.api.api.frame_presented)(self.token);
            (self.api.api.frame_release)(self.token, -1);
        }
    }
}

unsafe extern "C" fn destroy_state(context: *mut c_void) {
    if !context.is_null() {
        // SAFETY: allocated by Box::into_raw in Page::new exactly once.
        drop(unsafe { Box::from_raw(context.cast::<Rc<PageState>>()) });
    }
}

unsafe extern "C" fn event_callback(
    context: *mut c_void,
    kind: c_uint,
    first: *const c_char,
    _second: *const c_char,
    _number: c_double,
) {
    // SAFETY: context belongs to this live page.
    let state = unsafe { &*context.cast::<Rc<PageState>>() };
    let first = c_text(first);
    let event = match kind {
        1 => first.map(PageEvent::NavigationStarted),
        3 => Some(PageEvent::Loaded),
        5 | 6 => Some(PageEvent::LoadFailed(
            first.unwrap_or_else(|| "falha ao carregar WebEmbed".to_owned()),
        )),
        _ => None,
    };
    if let Some(event) = event {
        state.events.borrow_mut().push(event);
    }
}

unsafe extern "C" fn frame_callback(context: *mut c_void, frame: *const RawFrame) {
    if frame.is_null() {
        return;
    }
    // SAFETY: both pointers are supplied by the bridge for this callback.
    let state = unsafe { &*context.cast::<Rc<PageState>>() };
    let frame = unsafe { &*frame };
    if frame.n_planes != 1 || frame.fds[0] < 0 || frame.width == 0 || frame.height == 0 {
        return;
    }

    // SAFETY: the bridge transfers these duplicated descriptors to the consumer.
    let plane = unsafe { OwnedFd::from_raw_fd(frame.fds[0]) };
    let rendering_fence =
        (frame.rendering_fence_fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(frame.rendering_fence_fd) });

    state.frame.replace(Some(Frame {
        api: Arc::clone(&state.api),
        token: frame.token,
        width: frame.width,
        height: frame.height,
        format: frame.format,
        modifier: frame.modifier,
        plane,
        offset: frame.offsets[0],
        stride: frame.strides[0],
        rendering_fence,
        completed: false,
    }));
    if let Some(waker) = state.frame_waker.borrow().as_ref() {
        waker();
    }
}

unsafe extern "C" fn message_callback(
    _context: *mut c_void,
    _origin: *const c_char,
    _message: *const c_char,
) -> RawBytes {
    // Papo deliberately exposes no JS/native bridge to third-party embeds.
    RawBytes {
        data: std::ptr::null(),
        len: 0,
        user_data: std::ptr::null_mut(),
        destroy: None,
    }
}

fn c_string(value: &str, what: &str) -> Result<CString, String> {
    CString::new(value).map_err(|_| format!("{what} contém byte NUL"))
}

fn c_text(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: callbacks receive NUL-terminated strings owned for their duration.
    Some(
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn take_error(api: &RuntimeApi, error: *mut c_char) -> String {
    if error.is_null() {
        return "WPE falhou sem mensagem de erro".to_owned();
    }
    // SAFETY: bridge allocates one NUL-terminated error and supplies its free.
    let message = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: matching allocator for the pointer above.
    unsafe { (api.api.string_free)(error) };
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_path_defaults_next_to_executable() {
        let paths = RuntimePaths::discover().expect("current executable should resolve");
        assert!(paths.root().ends_with("waterui-browser/wpe") || std::env::var_os(RUNTIME_ENV).is_some());
    }

    #[test]
    fn c_string_rejects_nul() {
        assert!(c_string("https://example.com/\0x", "url").is_err());
    }
}
