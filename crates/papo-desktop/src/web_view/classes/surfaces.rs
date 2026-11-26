//! The virtual functions of Papo's WPE view and toplevel: a view hands each finished buffer to
//! its page (or straight back when its page is closing), forgets its buffers when WPE changes
//! them and passes on the cursor the page asks for; a toplevel resized by Papo resizes its
//! view. Safety: as `classes`, whose class layouts these mirror (sizes checked on load).

#![expect(
    unsafe_code,
    reason = "GObject virtual functions of WPEPlatform's C classes"
)]

use std::ffi::{c_char, c_int, c_uint, c_void};

use glib_sys::{GError, GFALSE, GTRUE, gboolean, gpointer};
use gobject_sys::GObjectClass;

use super::{delegate, page_of};
use crate::web_view::library::Object;

#[repr(C)]
pub(super) struct ViewClass {
    parent: GObjectClass,
    buffers_changed: Option<unsafe extern "C" fn(Object, *mut Object, c_uint)>,
    render_buffer: Option<
        unsafe extern "C" fn(Object, Object, *const c_void, c_uint, *mut *mut GError) -> gboolean,
    >,
    lock_pointer: Option<unsafe extern "C" fn(Object) -> gboolean>,
    unlock_pointer: Option<unsafe extern "C" fn(Object) -> gboolean>,
    set_cursor_from_name: Option<unsafe extern "C" fn(Object, *const c_char)>,
    set_cursor_from_bytes:
        Option<unsafe extern "C" fn(Object, *mut c_void, c_uint, c_uint, c_uint, c_uint, c_uint)>,
    set_opaque_rectangles: Option<unsafe extern "C" fn(Object, *mut c_void, c_uint)>,
    can_be_mapped: Option<unsafe extern "C" fn(Object) -> gboolean>,
    get_accessible: Option<unsafe extern "C" fn(Object) -> Object>,
    padding: [gpointer; 32],
}

#[repr(C)]
pub(super) struct ToplevelClass {
    parent: GObjectClass,
    set_title: Option<unsafe extern "C" fn(Object, *const c_char)>,
    get_screen: Option<unsafe extern "C" fn(Object) -> Object>,
    resize: Option<unsafe extern "C" fn(Object, c_int, c_int) -> gboolean>,
    set_fullscreen: Option<unsafe extern "C" fn(Object, gboolean) -> gboolean>,
    set_maximized: Option<unsafe extern "C" fn(Object, gboolean) -> gboolean>,
    set_minimized: Option<unsafe extern "C" fn(Object) -> gboolean>,
    get_preferred_buffer_formats: Option<unsafe extern "C" fn(Object) -> Object>,
    padding: [gpointer; 32],
}

pub(super) unsafe extern "C" fn view_class_init(class: gpointer, _data: gpointer) {
    // SAFETY: as `display_class_init`, for `ViewClass`.
    let class = unsafe { &mut *class.cast::<ViewClass>() };
    class.render_buffer = Some(view_render_buffer);
    class.buffers_changed = Some(view_buffers_changed);
    class.set_cursor_from_name = Some(view_cursor);
}

unsafe extern "C" fn view_render_buffer(
    view: Object,
    buffer: Object,
    _damage: *const c_void,
    _damage_count: c_uint,
    _error: *mut *mut GError,
) -> gboolean {
    let page = page_of(view);
    if page.is_null() {
        // A view whose page is closing: hand the buffer straight back.
        if let Some(delegate) = delegate() {
            // SAFETY: the view and buffer WPE passed.
            unsafe {
                (delegate.wpe.wpe_view_buffer_rendered)(view, buffer);
                (delegate.wpe.wpe_view_buffer_released)(view, buffer);
            }
        }
        return GTRUE;
    }
    // SAFETY: a stored page is alive until it clears itself from the view.
    unsafe { (*page).render(buffer) };
    GTRUE
}

unsafe extern "C" fn view_buffers_changed(view: Object, _buffers: *mut Object, _count: c_uint) {
    let page = page_of(view);
    if !page.is_null() {
        // SAFETY: as above.
        unsafe { (*page).forget_buffers() };
    }
}

unsafe extern "C" fn view_cursor(view: Object, name: *const c_char) {
    let page = page_of(view);
    if !page.is_null() {
        // SAFETY: as above; WPE passes a valid name.
        unsafe { (*page).cursor(&crate::web_view::glib::text(name)) };
    }
}

pub(super) unsafe extern "C" fn toplevel_class_init(class: gpointer, _data: gpointer) {
    // SAFETY: as `display_class_init`, for `ToplevelClass`.
    let class = unsafe { &mut *class.cast::<ToplevelClass>() };
    class.resize = Some(toplevel_resize);
}

/// A toplevel resized by Papo resizes its view too: without that WebKit keeps its default
/// 1024 × 768 viewport, shrinks the page into Papo's box and misplaces every pointer event.
unsafe extern "C" fn toplevel_resize(toplevel: Object, width: c_int, height: c_int) -> gboolean {
    let Some(delegate) = delegate() else {
        return GFALSE;
    };
    // SAFETY: the toplevel WPE passed and its views.
    unsafe {
        (delegate.wpe.wpe_toplevel_resized)(toplevel, width, height);
        (delegate.wpe.wpe_toplevel_foreach_view)(toplevel, resize_view, std::ptr::null_mut());
    }
    GTRUE
}

unsafe extern "C" fn resize_view(toplevel: Object, view: Object, _data: gpointer) -> gboolean {
    let Some(delegate) = delegate() else {
        return GTRUE;
    };
    let (mut width, mut height): (c_int, c_int) = (0, 0);
    // SAFETY: the toplevel and view WPE passed; the size is read into two ints.
    unsafe {
        (delegate.wpe.wpe_toplevel_get_size)(toplevel, &raw mut width, &raw mut height);
        if width > 0 && height > 0 {
            (delegate.wpe.wpe_view_resized)(view, width, height);
        }
    }
    GFALSE
}
