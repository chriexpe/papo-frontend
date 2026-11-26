//! WPEPlatform's display, view and toplevel, subclassed so WebKit renders into dmabufs Papo
//! draws (`docs/papo-desktop.md` §5): the display hands WebKit Papo's view and toplevel and
//! asks for linear ARGB8888 buffers on the GPU Rune draws on (R-013); the view passes each
//! finished buffer to its page; the toplevel resizes its view as Papo sizes the box.
//!
//! Owns the three types and their virtual functions. The display delegates EGL, the DRM
//! device, the keymap and screens to WPE's headless display on that GPU. What a page does with
//! a buffer is `frames`'.
//!
//! Safety: the class structs mirror WPE WebKit 2.x's headers (`WPEDisplayClass`,
//! `WPEViewClass`, `WPEToplevelClass`, each ending in 32 reserved pointers); [`types`] refuses
//! to register when the library's class sizes differ. The virtual functions run on the WPE
//! thread with the objects WPE passes them.

#![expect(unsafe_code, reason = "GObject subclasses of WPEPlatform's C classes")]

use std::cell::Cell;
use std::ffi::c_uint;
use std::sync::OnceLock;

use glib_sys::GType;
use glib_sys::{GError, GFALSE, GTRUE, gboolean, gpointer};
use gobject_sys::GObjectClass;

use super::glib::{Value, new_object};
use super::library::{Object, Unavailable, Wpe};
use super::page::Page;

mod surfaces;

use surfaces::{ToplevelClass, ViewClass, toplevel_class_init, view_class_init};

#[repr(C)]
struct DisplayClass {
    parent: GObjectClass,
    connect: Option<unsafe extern "C" fn(Object, *mut *mut GError) -> gboolean>,
    create_view: Option<unsafe extern "C" fn(Object) -> Object>,
    get_egl_display: Option<unsafe extern "C" fn(Object, *mut *mut GError) -> gpointer>,
    get_keymap: Option<unsafe extern "C" fn(Object) -> Object>,
    get_clipboard: Option<unsafe extern "C" fn(Object) -> Object>,
    get_preferred_buffer_formats: Option<unsafe extern "C" fn(Object) -> Object>,
    get_n_screens: Option<unsafe extern "C" fn(Object) -> c_uint>,
    get_screen: Option<unsafe extern "C" fn(Object, c_uint) -> Object>,
    get_drm_device: Option<unsafe extern "C" fn(Object) -> Object>,
    use_explicit_sync: Option<unsafe extern "C" fn(Object) -> gboolean>,
    create_input_method_context: Option<unsafe extern "C" fn(Object, Object) -> Object>,
    create_toplevel: Option<unsafe extern "C" fn(Object, c_uint) -> Object>,
    create_gamepad_manager: Option<unsafe extern "C" fn(Object) -> Object>,
    padding: [gpointer; 32],
}

/// Papo's three types.
#[derive(Clone, Copy)]
pub(super) struct Types {
    pub(super) display: GType,
}

/// What the display's virtual functions delegate to: set by the WPE thread before it makes
/// the display, read only there.
pub(super) struct Delegate {
    pub(super) wpe: &'static Wpe,
    headless: Object,
    formats: Object,
}

thread_local! {
    static DELEGATE: Cell<Option<&'static Delegate>> = const { Cell::new(None) };
}

static TYPES: OnceLock<Result<(Types, GType, GType), Unavailable>> = OnceLock::new();

/// Registers the types once per process, after checking the library's class sizes.
pub(super) fn types(wpe: &'static Wpe) -> Result<Types, Unavailable> {
    TYPES
        .get_or_init(|| register(wpe))
        .clone()
        .map(|(types, _, _)| types)
}

pub(super) fn view_type() -> GType {
    TYPES
        .get()
        .and_then(|types| types.as_ref().ok())
        .map_or(0, |types| types.1)
}

fn toplevel_type() -> GType {
    TYPES
        .get()
        .and_then(|types| types.as_ref().ok())
        .map_or(0, |types| types.2)
}

fn register(wpe: &'static Wpe) -> Result<(Types, GType, GType), Unavailable> {
    // SAFETY: the parent types come from the library; each class is registered once, with the
    // parent's own sizes after the check below.
    unsafe {
        let display = subclass(
            (wpe.wpe_display_get_type)(),
            c"PapoWpeDisplay",
            size_of::<DisplayClass>(),
            display_class_init,
        )?;
        let view = subclass(
            (wpe.wpe_view_get_type)(),
            c"PapoWpeView",
            size_of::<ViewClass>(),
            view_class_init,
        )?;
        let toplevel = subclass(
            (wpe.wpe_toplevel_get_type)(),
            c"PapoWpeToplevel",
            size_of::<ToplevelClass>(),
            toplevel_class_init,
        )?;
        Ok((Types { display }, view, toplevel))
    }
}

/// Registers `name` under `parent`, whose class must be `class_size` bytes.
unsafe fn subclass(
    parent: GType,
    name: &std::ffi::CStr,
    class_size: usize,
    init: unsafe extern "C" fn(gpointer, gpointer),
) -> Result<GType, Unavailable> {
    // SAFETY: `parent` is a registered type; the query fills a plain struct.
    let query = unsafe {
        let mut query: gobject_sys::GTypeQuery = std::mem::zeroed();
        gobject_sys::g_type_query(parent, &raw mut query);
        query
    };
    if query.class_size as usize != class_size {
        return Err(Unavailable(format!(
            "{}'s parent class is {} bytes, not the {class_size} this build knows",
            name.to_string_lossy(),
            query.class_size
        )));
    }
    // SAFETY: the sizes are the parent's own, so GObject copies its class and instance.
    let kind = unsafe {
        gobject_sys::g_type_register_static_simple(
            parent,
            name.as_ptr(),
            query.class_size,
            Some(init),
            query.instance_size,
            None,
            0,
        )
    };
    if kind == 0 {
        return Err(Unavailable(format!(
            "{} didn't register",
            name.to_string_lossy()
        )));
    }
    Ok(kind)
}

/// Makes the display: the headless one on the GPU at `device` (Rune's render node) or WPE's
/// choice, its buffer formats (linear ARGB8888 only, the one Rune imports), and Papo's display
/// over it, made primary with a mouse and keyboard (else WebKit reports `hover: none` and
/// `YouTube` falls back to its touch player). Returns Papo's display.
pub(super) fn make_display(
    wpe: &'static Wpe,
    types: Types,
    device: Option<&str>,
) -> Result<Object, Unavailable> {
    const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
    const DRM_FORMAT_MOD_LINEAR: u64 = 0;
    const USAGE_RENDERING: c_uint = 0;
    const INPUT_MOUSE_AND_KEYBOARD: c_uint = (1 << 0) | (1 << 1);
    // SAFETY: WPE's own constructors and setters on the objects they return, on this thread.
    unsafe {
        let mut error: *mut GError = std::ptr::null_mut();
        let headless = match device {
            Some(device) => {
                let device = super::glib::c_string(device);
                (wpe.wpe_display_headless_new_for_device)(device.as_ptr(), &raw mut error)
            }
            None => (wpe.wpe_display_headless_new)(),
        };
        if headless.is_null() {
            return Err(Unavailable(super::glib::take_error(error)));
        }
        if (wpe.wpe_display_connect)(headless, &raw mut error) == GFALSE {
            gobject_sys::g_object_unref(headless.cast());
            return Err(Unavailable(super::glib::take_error(error)));
        }
        let drm = (wpe.wpe_display_get_drm_device)(headless);
        let builder = (wpe.wpe_buffer_formats_builder_new)(drm);
        (wpe.wpe_buffer_formats_builder_append_group)(builder, drm, USAGE_RENDERING);
        (wpe.wpe_buffer_formats_builder_append_format)(
            builder,
            DRM_FORMAT_ARGB8888,
            DRM_FORMAT_MOD_LINEAR,
        );
        let formats = (wpe.wpe_buffer_formats_builder_end)(builder);
        (wpe.wpe_buffer_formats_builder_unref)(builder);
        let delegate: &'static Delegate = Box::leak(Box::new(Delegate {
            wpe,
            headless,
            formats,
        }));
        DELEGATE.set(Some(delegate));
        let display = new_object(types.display, &[]);
        (wpe.wpe_display_set_available_input_devices)(display, INPUT_MOUSE_AND_KEYBOARD);
        (wpe.wpe_display_set_primary)(display);
        Ok(display)
    }
}

/// Whether `view` is one of Papo's views (WebKit made it through Papo's display).
pub(super) fn is_papo_view(view: Object) -> bool {
    // SAFETY: a type check on a pointer WebKit returned, null included.
    !view.is_null()
        && unsafe { gobject_sys::g_type_check_instance_is_a(view.cast(), view_type()) } != GFALSE
}

/// A toplevel for one view on `display`; the caller owns it.
pub(super) fn new_toplevel(display: Object) -> Object {
    // SAFETY: Papo's toplevel type, as WPE's `create_toplevel` makes it.
    unsafe { display_create_toplevel(display, 1) }
}

/// The page `view` draws for, or null.
pub(super) fn page_of(view: Object) -> *const Page {
    // SAFETY: the quark's data is the page `set_page` stored, or null.
    unsafe { gobject_sys::g_object_get_qdata(view.cast(), page_quark()).cast::<Page>() }
}

/// Stores `page` (or null) on `view`, for its virtual functions.
pub(super) fn set_page(view: Object, page: *const Page) {
    // SAFETY: plain data; the page outlives the store (it clears it before it goes).
    unsafe {
        gobject_sys::g_object_set_qdata(view.cast(), page_quark(), page.cast_mut().cast());
    }
}

fn page_quark() -> glib_sys::GQuark {
    // SAFETY: a static string.
    unsafe { glib_sys::g_quark_from_static_string(c"papo-page".as_ptr()) }
}

pub(super) fn delegate() -> Option<&'static Delegate> {
    DELEGATE.get()
}

unsafe extern "C" fn display_class_init(class: gpointer, _data: gpointer) {
    // SAFETY: GObject passes this type's class, laid out as `DisplayClass` (size checked).
    let class = unsafe { &mut *class.cast::<DisplayClass>() };
    class.connect = Some(display_connect);
    class.create_view = Some(display_create_view);
    class.create_toplevel = Some(display_create_toplevel);
    class.get_egl_display = Some(display_egl);
    class.get_drm_device = Some(display_drm_device);
    class.get_keymap = Some(display_keymap);
    class.get_n_screens = Some(display_n_screens);
    class.get_screen = Some(display_screen);
    class.get_preferred_buffer_formats = Some(display_formats);
    class.use_explicit_sync = Some(display_explicit_sync);
}

unsafe extern "C" fn display_connect(_display: Object, _error: *mut *mut GError) -> gboolean {
    GTRUE
}

unsafe extern "C" fn display_create_view(display: Object) -> Object {
    // SAFETY: Papo's view type, with the display it belongs to.
    unsafe { new_object(view_type(), &[(c"display", Value::Object(display))]) }
}

unsafe extern "C" fn display_create_toplevel(display: Object, max_views: c_uint) -> Object {
    // SAFETY: Papo's toplevel type, with its display and its view count.
    unsafe {
        new_object(
            toplevel_type(),
            &[
                (c"display", Value::Object(display)),
                (c"max-views", Value::Uint(max_views)),
            ],
        )
    }
}

unsafe extern "C" fn display_egl(_display: Object, error: *mut *mut GError) -> gpointer {
    delegate().map_or(std::ptr::null_mut(), |delegate| {
        // SAFETY: the headless display is connected and lives for the process.
        unsafe { (delegate.wpe.wpe_display_get_egl_display)(delegate.headless, error) }
    })
}

unsafe extern "C" fn display_drm_device(_display: Object) -> Object {
    delegate().map_or(std::ptr::null_mut(), |delegate| {
        // SAFETY: as above.
        unsafe { (delegate.wpe.wpe_display_get_drm_device)(delegate.headless) }
    })
}

unsafe extern "C" fn display_keymap(_display: Object) -> Object {
    delegate().map_or(std::ptr::null_mut(), |delegate| {
        // SAFETY: as above.
        unsafe { (delegate.wpe.wpe_display_get_keymap)(delegate.headless) }
    })
}

unsafe extern "C" fn display_n_screens(_display: Object) -> c_uint {
    delegate().map_or(0, |delegate| {
        // SAFETY: as above.
        unsafe { (delegate.wpe.wpe_display_get_n_screens)(delegate.headless) }
    })
}

unsafe extern "C" fn display_screen(_display: Object, index: c_uint) -> Object {
    delegate().map_or(std::ptr::null_mut(), |delegate| {
        // SAFETY: as above.
        unsafe { (delegate.wpe.wpe_display_get_screen)(delegate.headless, index) }
    })
}

unsafe extern "C" fn display_formats(_display: Object) -> Object {
    delegate().map_or(std::ptr::null_mut(), |delegate| delegate.formats)
}

unsafe extern "C" fn display_explicit_sync(_display: Object) -> gboolean {
    // Each buffer comes with its rendering fence, which `frames` waits on before Rune reads.
    GTRUE
}
