//! What a provider page may do (`docs/design.md` §15.3): it stays on its player. A navigation
//! elsewhere, or a new window, opens in the browser when the person clicked for it and is
//! refused otherwise; downloads, file choosers, script dialogs and every permission (camera,
//! microphone, location, notifications) are refused; the provider's full-screen request opens
//! Papo's viewer instead of taking the window. Also reports loading, failure and sound.
//!
//! Owns the page's signal handlers. Whether an address is still the player is the SDK's
//! (`ProviderPlayer::keeps`); opening the browser goes through Papo's link rules (§9).
//!
//! Safety: each handler's data is its live, boxed page (see `page`), and its signature is the
//! signal's in WebKit 2.x; handlers run on the WPE thread.

#![expect(unsafe_code, reason = "WebKit signal handlers")]

use std::ffi::{CStr, c_char, c_uint, c_void};

use glib_sys::{GError, GFALSE, GTRUE, gboolean, gpointer};

use super::glib::{connect as connect_signal, disconnect_all, text};
use super::library::Object;
use super::page::Page;
use super::{PageEvent, WebEvent};

const LOAD_COMMITTED: c_uint = 2;
const DECISION_NAVIGATION: c_uint = 0;
const DECISION_NEW_WINDOW: c_uint = 1;
const DECISION_RESPONSE: c_uint = 2;
/// `WEBKIT_NETWORK_ERROR_CANCELLED`: a load Papo or the page stopped, not a failure.
const NETWORK_ERROR_CANCELLED: i32 = 302;

/// Connects the page's handlers.
pub(super) fn connect(page: &Page) {
    let data = std::ptr::from_ref(page).cast_mut().cast::<c_void>();
    let handlers: [(&CStr, unsafe extern "C" fn()); 11] = [
        (
            c"load-changed",
            callback(load_changed as unsafe extern "C" fn(Object, c_uint, gpointer)),
        ),
        (
            c"load-failed",
            callback(
                load_failed
                    as unsafe extern "C" fn(
                        Object,
                        c_uint,
                        *const c_char,
                        *mut GError,
                        gpointer,
                    ) -> gboolean,
            ),
        ),
        (
            c"load-failed-with-tls-errors",
            callback(
                tls_failed
                    as unsafe extern "C" fn(
                        Object,
                        *const c_char,
                        Object,
                        c_uint,
                        gpointer,
                    ) -> gboolean,
            ),
        ),
        (
            c"web-process-terminated",
            callback(terminated as unsafe extern "C" fn(Object, c_uint, gpointer)),
        ),
        (
            c"decide-policy",
            callback(decide as unsafe extern "C" fn(Object, Object, c_uint, gpointer) -> gboolean),
        ),
        (
            c"permission-request",
            callback(
                refuse_permission as unsafe extern "C" fn(Object, Object, gpointer) -> gboolean,
            ),
        ),
        (
            c"create",
            callback(no_window as unsafe extern "C" fn(Object, Object, gpointer) -> Object),
        ),
        (
            c"enter-fullscreen",
            callback(full_screen as unsafe extern "C" fn(Object, gpointer) -> gboolean),
        ),
        (
            c"run-file-chooser",
            callback(
                refuse_file_chooser as unsafe extern "C" fn(Object, Object, gpointer) -> gboolean,
            ),
        ),
        (
            c"script-dialog",
            callback(refuse_dialog as unsafe extern "C" fn(Object, Object, gpointer) -> gboolean),
        ),
        (
            c"notify::is-playing-audio",
            callback(sound_changed as unsafe extern "C" fn(Object, *mut c_void, gpointer)),
        ),
    ];
    for (signal, handler) in handlers {
        // SAFETY: each handler's signature is its signal's; the page outlives the connection
        // (it disconnects before it goes).
        unsafe { connect_signal(page.web_view, signal, Some(handler), data) };
    }
}

/// Disconnects every handler of the page.
pub(super) fn disconnect(page: &Page) {
    let data = std::ptr::from_ref(page).cast_mut().cast::<c_void>();
    // SAFETY: the page's own view.
    unsafe { disconnect_all(page.web_view, data) };
}

/// `handler` as GObject's untyped callback.
fn callback<F: Copy>(handler: F) -> unsafe extern "C" fn() {
    debug_assert_eq!(size_of::<F>(), size_of::<unsafe extern "C" fn()>());
    // SAFETY: `F` is an `extern "C"` function pointer; GObject calls it back with the
    // signal's own arguments, which are the ones it was written for.
    unsafe { std::mem::transmute_copy::<F, unsafe extern "C" fn()>(&handler) }
}

/// The page `data` names.
///
/// # Safety
/// `data` is a page connected by [`connect`] and not yet disconnected.
unsafe fn page<'a>(data: gpointer) -> &'a Page {
    // SAFETY: see the contract.
    unsafe { &*data.cast::<Page>() }
}

fn tell(page: &Page, event: PageEvent) {
    (page.events)(WebEvent::Page(page.id, event));
}

unsafe extern "C" fn load_changed(_web_view: Object, event: c_uint, data: gpointer) {
    if event == LOAD_COMMITTED {
        // SAFETY: see the module header.
        unsafe { page(data) }.committed.set(true);
    }
}

unsafe extern "C" fn load_failed(
    _web_view: Object,
    _event: c_uint,
    _address: *const c_char,
    error: *mut GError,
    data: gpointer,
) -> gboolean {
    // SAFETY: see the module header; WebKit's error is borrowed for the call.
    let (page, code) = unsafe { (page(data), error.as_ref().map_or(0, |error| error.code)) };
    // Once the player's document is in, a later failure is a navigation the policy refused:
    // the player stays as it is.
    if !page.committed.get() && code != NETWORK_ERROR_CANCELLED {
        tell(page, PageEvent::Failed);
    }
    // Handled: WebKit draws no error page of its own in the box.
    GTRUE
}

unsafe extern "C" fn tls_failed(
    _web_view: Object,
    _address: *const c_char,
    _certificate: Object,
    _errors: c_uint,
    data: gpointer,
) -> gboolean {
    // SAFETY: see the module header.
    tell(unsafe { page(data) }, PageEvent::Failed);
    GFALSE
}

unsafe extern "C" fn terminated(_web_view: Object, _reason: c_uint, data: gpointer) {
    // SAFETY: see the module header.
    tell(unsafe { page(data) }, PageEvent::Failed);
}

unsafe extern "C" fn decide(
    _web_view: Object,
    decision: Object,
    kind: c_uint,
    data: gpointer,
) -> gboolean {
    // SAFETY: see the module header; the decision and what it holds are WebKit's, live for
    // the call, and answered exactly once.
    unsafe {
        let page = page(data);
        let wpe = page.wpe;
        let allowed = match kind {
            DECISION_NAVIGATION | DECISION_NEW_WINDOW => {
                let action =
                    (wpe.webkit_navigation_policy_decision_get_navigation_action)(decision);
                let request = (wpe.webkit_navigation_action_get_request)(action);
                let address = text((wpe.webkit_uri_request_get_uri)(request));
                let clicked = (wpe.webkit_navigation_action_is_user_gesture)(action) != GFALSE;
                let stays = kind == DECISION_NAVIGATION && (page.keeps)(&address);
                if !stays && clicked {
                    tell(page, PageEvent::OpenInBrowser(address));
                }
                // A navigation the person didn't ask for may be a frame inside the player
                // (the response check below keeps the page itself on its player); a new
                // window never opens.
                stays || (kind == DECISION_NAVIGATION && !clicked)
            }
            DECISION_RESPONSE => {
                let main =
                    (wpe.webkit_response_policy_decision_is_main_frame_main_resource)(decision)
                        != GFALSE;
                let request = (wpe.webkit_response_policy_decision_get_request)(decision);
                let address = text((wpe.webkit_uri_request_get_uri)(request));
                let shown = (wpe.webkit_response_policy_decision_is_mime_type_supported)(decision)
                    != GFALSE;
                // A download is a response no page shows.
                shown && (!main || (page.keeps)(&address))
            }
            _ => false,
        };
        if allowed {
            (wpe.webkit_policy_decision_use)(decision);
        } else {
            (wpe.webkit_policy_decision_ignore)(decision);
        }
    }
    GTRUE
}

unsafe extern "C" fn refuse_permission(
    _web_view: Object,
    request: Object,
    data: gpointer,
) -> gboolean {
    // SAFETY: see the module header; the request is answered once.
    unsafe { (page(data).wpe.webkit_permission_request_deny)(request) };
    GTRUE
}

unsafe extern "C" fn no_window(_web_view: Object, _action: Object, _data: gpointer) -> Object {
    std::ptr::null_mut()
}

unsafe extern "C" fn full_screen(_web_view: Object, data: gpointer) -> gboolean {
    // SAFETY: see the module header.
    tell(unsafe { page(data) }, PageEvent::FullScreen);
    // Handled: the page stays in its box, which Papo's viewer takes over.
    GTRUE
}

unsafe extern "C" fn refuse_file_chooser(
    _web_view: Object,
    request: Object,
    data: gpointer,
) -> gboolean {
    // SAFETY: see the module header; the request is answered once.
    unsafe { (page(data).wpe.webkit_file_chooser_request_cancel)(request) };
    GTRUE
}

unsafe extern "C" fn refuse_dialog(
    _web_view: Object,
    _dialog: Object,
    _data: gpointer,
) -> gboolean {
    // Handled with nothing shown: an alert is dismissed, a confirm or prompt declined.
    GTRUE
}

unsafe extern "C" fn sound_changed(web_view: Object, _spec: *mut c_void, data: gpointer) {
    // SAFETY: see the module header.
    let page = unsafe { page(data) };
    // SAFETY: the page's view.
    let playing = unsafe { (page.wpe.webkit_web_view_is_playing_audio)(web_view) } != GFALSE;
    tell(page, PageEvent::Sound(playing));
}
