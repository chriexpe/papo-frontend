//! The few GLib and GObject calls the players make, made safe to use from the rest of
//! `web_view`: objects made with construct properties, a signal
//! connected to a C handler, a `GError` read into text, and a closure run on the WPE thread's
//! main context from any thread.
//!
//! Owns no WPE state. Safety: every function takes pointers to live GObjects of the type its
//! documentation names; callers on the WPE thread guarantee that.

#![expect(unsafe_code, reason = "GLib and GObject C calls")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;

use glib_sys::GType;
use glib_sys::{GError, GFALSE, GMainContext, gpointer};
use gobject_sys::{GCallback, GObject, GValue};

/// A construct property's value, for [`new_object`].
pub(crate) enum Value {
    /// An object (`G_TYPE_OBJECT` or a subtype).
    Object(*mut c_void),
    Uint(u32),
    /// An enum's member, of the enum's type.
    Enum(GType, i32),
}

/// `value` as an initialised `GValue`; the caller unsets it.
fn gvalue(value: &Value) -> GValue {
    // SAFETY: a zeroed GValue is G_VALUE_INIT; each init names the type its setter writes.
    unsafe {
        let mut out: GValue = std::mem::zeroed();
        match value {
            Value::Object(object) => {
                gobject_sys::g_value_init(&raw mut out, gobject_sys::G_TYPE_OBJECT);
                gobject_sys::g_value_set_object(&raw mut out, object.cast());
            }
            Value::Uint(number) => {
                gobject_sys::g_value_init(&raw mut out, gobject_sys::G_TYPE_UINT);
                gobject_sys::g_value_set_uint(&raw mut out, *number);
            }
            Value::Enum(kind, member) => {
                gobject_sys::g_value_init(&raw mut out, *kind);
                gobject_sys::g_value_set_enum(&raw mut out, *member);
            }
        }
        out
    }
}

/// A new object of `kind` with its construct `properties`; the caller owns the reference.
///
/// # Safety
/// `kind` is a registered object type and each property is one of its own, of the value's
/// type.
pub(crate) unsafe fn new_object(kind: GType, properties: &[(&CStr, Value)]) -> *mut c_void {
    let names: Vec<*const c_char> = properties.iter().map(|(name, _)| name.as_ptr()).collect();
    let mut values: Vec<GValue> = properties.iter().map(|(_, value)| gvalue(value)).collect();
    let count = u32::try_from(properties.len()).unwrap_or(0);
    // SAFETY: names and values are `count` long and live across the call (see the contract).
    unsafe {
        let object = gobject_sys::g_object_new_with_properties(
            kind,
            count,
            names.as_ptr().cast_mut(),
            values.as_ptr(),
        );
        for value in &mut values {
            gobject_sys::g_value_unset(value);
        }
        object.cast()
    }
}

/// Connects `handler` to `object`'s `signal` with `data`, returning the handler's id.
///
/// # Safety
/// `handler` is an `extern "C"` function whose signature is the signal's, cast to
/// `GCallback`; `data` stays valid until the handler is disconnected.
pub(crate) unsafe fn connect(
    object: *mut c_void,
    signal: &CStr,
    handler: GCallback,
    data: gpointer,
) -> u64 {
    // SAFETY: see the contract.
    unsafe {
        gobject_sys::g_signal_connect_data(
            object.cast::<GObject>(),
            signal.as_ptr(),
            handler,
            data,
            None,
            0,
        )
    }
}

/// Disconnects every handler `object` has with `data`.
///
/// # Safety
/// `object` is a live GObject.
pub(crate) unsafe fn disconnect_all(object: *mut c_void, data: gpointer) {
    // SAFETY: matching on data alone touches only this object's handlers.
    unsafe {
        gobject_sys::g_signal_handlers_disconnect_matched(
            object.cast::<GObject>(),
            gobject_sys::G_SIGNAL_MATCH_DATA,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            data,
        );
    }
}

/// The text of `error`, which this frees; empty for no error.
///
/// # Safety
/// `error` is null or a `GError` the caller owns.
pub(crate) unsafe fn take_error(error: *mut GError) -> String {
    if error.is_null() {
        return String::new();
    }
    // SAFETY: a GError's message is a valid C string until the error is freed.
    unsafe {
        let text = CStr::from_ptr((*error).message)
            .to_string_lossy()
            .into_owned();
        glib_sys::g_error_free(error);
        text
    }
}

/// `text` as a C string, with any NUL cut out (an address never has one).
pub(crate) fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// `text`, a C string WebKit owns, as Rust text; empty when null.
///
/// # Safety
/// `text` is null or a valid C string.
pub(crate) unsafe fn text(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: see the contract.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

/// A closure for the WPE thread.
type Job = Box<dyn FnOnce() + Send>;

/// A thread's main context, to which other threads hand closures. Each `Context` holds its
/// own reference, so a hand-back that outlives the WebView thread still invokes on a live
/// context (its job is then dropped unrun when the last reference goes).
pub(crate) struct Context(*mut GMainContext);

// SAFETY: a GMainContext is thread-safe and reference-counted atomically;
// `g_main_context_invoke_full` is the call made from other threads, and every `Context`
// holds a reference of its own.
unsafe impl Send for Context {}
// SAFETY: as above.
unsafe impl Sync for Context {}

impl Context {
    /// A new main context, for the WebView thread.
    pub(crate) fn new() -> Context {
        // SAFETY: a new context, whose reference this takes.
        Context(unsafe { glib_sys::g_main_context_new() })
    }

    pub(crate) fn raw(&self) -> *mut GMainContext {
        self.0
    }

    /// Runs `job` on the context's thread, at the next turn of its loop. A job whose loop
    /// has stopped is dropped there without running.
    pub(crate) fn invoke(&self, job: impl FnOnce() + Send + 'static) {
        let job: *mut Option<Job> = Box::into_raw(Box::new(Some(Box::new(job))));
        // SAFETY: this holds a reference to the context; `run` and `drop_job` take the box
        // back exactly once, the second after the first.
        unsafe {
            glib_sys::g_main_context_invoke_full(
                self.0,
                glib_sys::G_PRIORITY_DEFAULT,
                Some(run),
                job.cast(),
                Some(drop_job),
            );
        }
    }
}

impl Clone for Context {
    fn clone(&self) -> Context {
        // SAFETY: a live context (this holds a reference); the clone takes its own.
        Context(unsafe { glib_sys::g_main_context_ref(self.0) })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the reference this held.
        unsafe { glib_sys::g_main_context_unref(self.0) };
    }
}

unsafe extern "C" fn run(job: gpointer) -> glib_sys::gboolean {
    // SAFETY: `job` is the box `invoke` made; it is freed by `drop_job`, never here.
    if let Some(job) = unsafe { (*job.cast::<Option<Job>>()).take() } {
        job();
    }
    GFALSE
}

unsafe extern "C" fn drop_job(job: gpointer) {
    // SAFETY: the box `invoke` made, released once, after `run` (if it ran).
    drop(unsafe { Box::from_raw(job.cast::<Option<Job>>()) });
}
