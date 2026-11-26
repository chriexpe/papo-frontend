//! The system's WPE WebKit, opened when the app runs: `libWPEWebKit-2.0.so.1` (which also
//! holds WPEPlatform) and the `libsoup-3.0.so.0` it already loads, with every function the
//! players call resolved up front. A library that is missing, or lacks one of them, is
//! [`Unavailable`]: the players stay off (`docs/design.md` §15.7) and nothing else changes.
//!
//! Owns the symbol table and the ABI checks; what the functions are called for is the rest of
//! `web_view`'s. GLib and GObject themselves are linked (`glib-sys`, `gobject-sys`): GStreamer
//! already loads them, and WPE shares the one copy the process has.
//!
//! Safety: each pointer here is the address of the named C function, typed after WPE WebKit
//! 2.x's headers (the version checked on load). The libraries are kept open for as long as the
//! table exists, so the pointers stay valid.

#![expect(
    unsafe_code,
    reason = "resolving C functions from a library opened at run time"
)]

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::sync::OnceLock;

use glib_sys::GType;
use glib_sys::{GError, GList, gboolean, gpointer};
use libloading::Library;

/// The WPE WebKit library, by its soname (the 2.0 API).
const WPE_WEBKIT: &str = "libWPEWebKit-2.0.so.1";
/// libsoup 3, for the request's headers.
const SOUP: &str = "libsoup-3.0.so.0";
/// The oldest WPE WebKit with every function and class this module uses (WPEPlatform's
/// headless display on a chosen device).
const OLDEST_MINOR: u32 = 50;

/// Why the players can't load here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Unavailable(pub(crate) String);

/// An opaque WPE or WebKit object.
pub(crate) type Object = *mut c_void;

/// A `WPEKeymapEntry`.
#[repr(C)]
pub(crate) struct KeymapEntry {
    pub(crate) keycode: c_uint,
    pub(crate) group: c_int,
    pub(crate) level: c_int,
}

/// `WPEToplevelForeachViewFunc`.
pub(crate) type ForeachView = unsafe extern "C" fn(Object, Object, gpointer) -> gboolean;
/// `GAsyncReadyCallback`.
pub(crate) type AsyncReady = unsafe extern "C" fn(*mut gobject_sys::GObject, *mut c_void, gpointer);

macro_rules! symbols {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// The functions the players call, resolved from the opened libraries.
        pub(crate) struct Wpe {
            $(pub(crate) $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            pub(crate) soup_message_headers_replace:
                unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),
            _soup: Library,
            _webkit: Library,
        }

        impl Wpe {
            fn resolve(webkit: Library, soup: Library) -> Result<Wpe, Unavailable> {
                // SAFETY: every name is a C function of WPE WebKit 2.x or libsoup 3 with the
                // signature written beside it; the libraries move into the table, so the
                // pointers outlive neither.
                unsafe {
                    Ok(Wpe {
                        $($name: *webkit
                            .get(concat!(stringify!($name), "\0").as_bytes())
                            .map_err(|error| Unavailable(error.to_string()))?,)*
                        soup_message_headers_replace: *soup
                            .get(b"soup_message_headers_replace\0")
                            .map_err(|error| Unavailable(error.to_string()))?,
                        _soup: soup,
                        _webkit: webkit,
                    })
                }
            }
        }
    };
}

symbols! {
    webkit_get_major_version: fn() -> c_uint;
    webkit_get_minor_version: fn() -> c_uint;
    // WPEPlatform's types and the headless display it delegates to.
    wpe_display_get_type: fn() -> GType;
    wpe_view_get_type: fn() -> GType;
    wpe_toplevel_get_type: fn() -> GType;
    wpe_buffer_dma_buf_get_type: fn() -> GType;
    wpe_display_headless_new: fn() -> Object;
    wpe_display_headless_new_for_device: fn(*const c_char, *mut *mut GError) -> Object;
    wpe_display_connect: fn(Object, *mut *mut GError) -> gboolean;
    wpe_display_get_egl_display: fn(Object, *mut *mut GError) -> gpointer;
    wpe_display_get_drm_device: fn(Object) -> Object;
    wpe_display_get_keymap: fn(Object) -> Object;
    wpe_display_get_n_screens: fn(Object) -> c_uint;
    wpe_display_get_screen: fn(Object, c_uint) -> Object;
    wpe_display_set_available_input_devices: fn(Object, c_uint);
    wpe_display_set_primary: fn(Object);
    wpe_buffer_formats_builder_new: fn(Object) -> Object;
    wpe_buffer_formats_builder_append_group: fn(Object, Object, c_uint);
    wpe_buffer_formats_builder_append_format: fn(Object, u32, u64);
    wpe_buffer_formats_builder_end: fn(Object) -> Object;
    wpe_buffer_formats_builder_unref: fn(Object);
    // Toplevels and views.
    wpe_toplevel_resized: fn(Object, c_int, c_int);
    wpe_toplevel_resize: fn(Object, c_int, c_int) -> gboolean;
    wpe_toplevel_scale_changed: fn(Object, f64);
    wpe_toplevel_state_changed: fn(Object, c_uint);
    wpe_toplevel_foreach_view: fn(Object, ForeachView, gpointer);
    wpe_toplevel_closed: fn(Object);
    wpe_toplevel_get_size: fn(Object, *mut c_int, *mut c_int);
    wpe_view_resized: fn(Object, c_int, c_int);
    wpe_view_map: fn(Object);
    wpe_view_set_toplevel: fn(Object, Object);
    wpe_view_set_visible: fn(Object, gboolean);
    wpe_view_closed: fn(Object);
    wpe_view_focus_in: fn(Object);
    wpe_view_focus_out: fn(Object);
    wpe_view_event: fn(Object, Object);
    wpe_view_compute_press_count: fn(Object, f64, f64, c_uint, u32) -> c_uint;
    wpe_view_buffer_rendered: fn(Object, Object);
    wpe_view_buffer_released: fn(Object, Object);
    // Buffers.
    wpe_buffer_get_width: fn(Object) -> c_int;
    wpe_buffer_get_height: fn(Object) -> c_int;
    wpe_buffer_take_rendering_fence: fn(Object) -> c_int;
    wpe_buffer_dma_buf_get_n_planes: fn(Object) -> u32;
    wpe_buffer_dma_buf_get_format: fn(Object) -> u32;
    wpe_buffer_dma_buf_get_modifier: fn(Object) -> u64;
    wpe_buffer_dma_buf_get_fd: fn(Object, u32) -> c_int;
    wpe_buffer_dma_buf_get_offset: fn(Object, u32) -> u32;
    wpe_buffer_dma_buf_get_stride: fn(Object, u32) -> u32;
    // Input.
    wpe_event_pointer_button_new:
        fn(c_uint, Object, c_uint, u32, c_uint, c_uint, f64, f64, c_uint) -> Object;
    wpe_event_pointer_move_new: fn(c_uint, Object, c_uint, u32, c_uint, f64, f64, f64, f64) -> Object;
    wpe_event_scroll_new: fn(Object, c_uint, u32, c_uint, f64, f64, gboolean, gboolean, f64, f64) -> Object;
    wpe_event_keyboard_new: fn(c_uint, Object, c_uint, u32, c_uint, c_uint, c_uint) -> Object;
    wpe_event_unref: fn(Object);
    wpe_unicode_to_keyval: fn(u32) -> c_uint;
    wpe_keymap_get_entries_for_keyval: fn(Object, c_uint, *mut *mut KeymapEntry, *mut c_uint) -> gboolean;
    // WebKit: the view, its settings and policies, its network session.
    webkit_web_view_get_type: fn() -> GType;
    webkit_website_policies_get_type: fn() -> GType;
    webkit_autoplay_policy_get_type: fn() -> GType;
    webkit_web_view_get_wpe_view: fn(Object) -> Object;
    webkit_web_view_get_settings: fn(Object) -> Object;
    webkit_settings_set_user_agent: fn(Object, *const c_char);
    webkit_web_view_load_request: fn(Object, Object);
    webkit_web_view_stop_loading: fn(Object);
    webkit_web_view_set_is_muted: fn(Object, gboolean);
    webkit_web_view_is_playing_audio: fn(Object) -> gboolean;
    webkit_uri_request_new: fn(*const c_char) -> Object;
    webkit_uri_request_get_uri: fn(Object) -> *const c_char;
    webkit_uri_request_get_http_headers: fn(Object) -> *mut c_void;
    webkit_network_session_new: fn(*const c_char, *const c_char) -> Object;
    webkit_network_session_get_website_data_manager: fn(Object) -> Object;
    webkit_network_session_get_cookie_manager: fn(Object) -> Object;
    webkit_cookie_manager_set_persistent_storage: fn(Object, *const c_char, c_uint);
    webkit_website_data_manager_fetch: fn(Object, c_uint, *mut c_void, AsyncReady, gpointer);
    webkit_website_data_manager_fetch_finish: fn(Object, *mut c_void, *mut *mut GError) -> *mut GList;
    webkit_website_data_manager_clear: fn(Object, c_uint, i64, *mut c_void, AsyncReady, gpointer);
    webkit_website_data_manager_clear_finish: fn(Object, *mut c_void, *mut *mut GError) -> gboolean;
    webkit_website_data_unref: fn(*mut c_void);
    // What a page asks for, answered.
    webkit_policy_decision_use: fn(Object);
    webkit_policy_decision_ignore: fn(Object);
    webkit_navigation_policy_decision_get_navigation_action: fn(Object) -> Object;
    webkit_navigation_action_get_request: fn(Object) -> Object;
    webkit_navigation_action_is_user_gesture: fn(Object) -> gboolean;
    webkit_response_policy_decision_is_mime_type_supported: fn(Object) -> gboolean;
    webkit_response_policy_decision_is_main_frame_main_resource: fn(Object) -> gboolean;
    webkit_response_policy_decision_get_request: fn(Object) -> Object;
    webkit_permission_request_deny: fn(Object);
    webkit_download_cancel: fn(Object);
    webkit_file_chooser_request_cancel: fn(Object);
}

/// The libraries, opened the first time they are asked for and kept for the process (WebKit
/// can't be unloaded safely).
pub(crate) fn loaded() -> Result<&'static Wpe, Unavailable> {
    static WPE: OnceLock<Result<Wpe, Unavailable>> = OnceLock::new();
    WPE.get_or_init(Wpe::open).as_ref().map_err(Clone::clone)
}

impl Wpe {
    /// Opens the libraries and resolves every function, or says why the players can't load.
    fn open() -> Result<Wpe, Unavailable> {
        // SAFETY: opening runs the libraries' initialisers, which are WebKit's and libsoup's
        // own; nothing of ours runs before they finish.
        let webkit = unsafe { Library::new(WPE_WEBKIT) }
            .map_err(|error| Unavailable(format!("{WPE_WEBKIT}: {error}")))?;
        // SAFETY: as above; WebKit has already loaded it, so this only takes a reference.
        let soup = unsafe { Library::new(SOUP) }
            .map_err(|error| Unavailable(format!("{SOUP}: {error}")))?;
        let wpe = Wpe::resolve(webkit, soup)?;
        // SAFETY: plain version queries.
        let (major, minor) = unsafe {
            (
                (wpe.webkit_get_major_version)(),
                (wpe.webkit_get_minor_version)(),
            )
        };
        if major != 2 || minor < OLDEST_MINOR {
            return Err(Unavailable(format!(
                "WPE WebKit {major}.{minor} is older than 2.{OLDEST_MINOR}"
            )));
        }
        Ok(wpe)
    }
}
