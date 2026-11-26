//! One provider page in WPE WebKit (`docs/design.md` §15.3): its `WebKitWebView`, the WPE view
//! and toplevel it draws through, its size, the input Papo forwards and its sound. Owns the
//! page's objects from opening to closing; what the page may do (`policy`), its frames
//! (`frames`) and its site's storage (`sites`) are the neighbours'.
//!
//! Safety: lives on the WPE thread. A page is boxed and never moves: its address is the data
//! of its view and signal handlers, which it clears and disconnects before it lets its objects
//! go.

#![expect(unsafe_code, reason = "WebKit's and WPEPlatform's C objects")]

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::Arc;

use glib_sys::{GFALSE, GTRUE};

use super::classes;
use super::frames::Frames;
use super::glib::{Context, Value, c_string, new_object};
use super::library::{Object, Wpe};
use super::{Events, PageEvent, PageId, PageSpec, WebEvent};

mod input;

/// What `YouTube` and other players are told the browser is: a Chrome on Linux, as the
/// Chromium-based clients present themselves, so the player offers its desktop controls.
const USER_AGENT: &std::ffi::CStr = c"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
/// The page that embeds the player, as players ask (`YouTube` refuses to play without one):
/// Papo's own identity, never a real site.
const REFERER: &std::ffi::CStr = c"https://io.github.chriexpe.papo/";

const AUTOPLAY_ALLOW: i32 = 0;
const TOPLEVEL_ACTIVE: u32 = 1 << 2;

/// A provider page.
pub(super) struct Page {
    pub(super) id: PageId,
    pub(super) wpe: &'static Wpe,
    pub(super) web_view: Object,
    pub(super) view: Object,
    toplevel: Object,
    pub(super) keymap: Object,
    frames: Frames,
    /// Whether an address is still this player (the SDK's `ProviderPlayer::keeps`).
    pub(super) keeps: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub(super) events: Events,
    /// The player's document has started to load: later failures (a navigation the policy
    /// refused) leave it as it is.
    pub(super) committed: Cell<bool>,
    /// The address of the last navigation the person clicked for that leaves the player, until
    /// a response shows where it went (`policy`).
    pub(super) clicked: std::cell::RefCell<Option<String>>,
    painted: Cell<bool>,
    /// WPE's bits for the pointer buttons held.
    pub(super) buttons: Cell<u32>,
}

/// What every page on the WebView thread opens with.
pub(super) struct Shared {
    pub(super) wpe: &'static Wpe,
    /// The thread's main context, where hand-backs return.
    pub(super) context: Context,
    pub(super) display: Object,
    /// Papo's own WebKit context.
    pub(super) web_context: Object,
}

impl Page {
    /// Opens `spec`'s address with `shared`'s display and WebKit context, storing its site
    /// data in `session`.
    pub(super) fn open(
        shared: Shared,
        session: Object,
        id: PageId,
        spec: PageSpec,
        events: Events,
    ) -> Result<NonNull<Page>, String> {
        let Shared {
            wpe,
            context,
            display,
            web_context,
        } = shared;
        // SAFETY: WebKit's constructors and getters, on the WPE thread; every object made
        // here is owned by the page or released before returning.
        let (web_view, view, keymap) = unsafe {
            let policies = new_object(
                (wpe.webkit_website_policies_get_type)(),
                &[(
                    c"autoplay",
                    // Players load only after a press on Play, which asked them to play.
                    Value::Enum((wpe.webkit_autoplay_policy_get_type)(), AUTOPLAY_ALLOW),
                )],
            );
            let web_view = new_object(
                (wpe.webkit_web_view_get_type)(),
                &[
                    (c"display", Value::Object(display)),
                    // Papo's own context, never WebKit's process-wide default: that one is a
                    // static WebKit destroys at exit, on the main thread rather than this one.
                    (c"web-context", Value::Object(web_context)),
                    (c"network-session", Value::Object(session)),
                    (c"website-policies", Value::Object(policies)),
                ],
            );
            gobject_sys::g_object_unref(policies.cast());
            if web_view.is_null() {
                return Err("WebKit made no view".into());
            }
            let settings = (wpe.webkit_web_view_get_settings)(web_view);
            (wpe.webkit_settings_set_user_agent)(settings, USER_AGENT.as_ptr());
            let view = (wpe.webkit_web_view_get_wpe_view)(web_view);
            if !classes::is_papo_view(view) {
                gobject_sys::g_object_unref(web_view.cast());
                return Err("WebKit didn't make Papo's view".into());
            }
            (web_view, view, (wpe.wpe_display_get_keymap)(display))
        };
        let frames = Frames::new(wpe, context, spec.surface.clone());
        // Leaked here and taken back by `close` only: its address is the data of its view and
        // signal handlers, so the box must never move while they can run.
        let page: &Page = Box::leak(Box::new(Page {
            id,
            wpe,
            web_view,
            view,
            toplevel: classes::new_toplevel(display),
            keymap,
            frames,
            keeps: spec.keeps,
            events,
            committed: Cell::new(false),
            clicked: std::cell::RefCell::new(None),
            painted: Cell::new(false),
            buttons: Cell::new(0),
        }));
        classes::set_page(view, page);
        // SAFETY: the page's own objects; the toplevel was just made for it.
        unsafe {
            (wpe.wpe_view_set_toplevel)(view, page.toplevel);
            (wpe.wpe_toplevel_state_changed)(page.toplevel, TOPLEVEL_ACTIVE);
        }
        page.resize(spec.size_pt, spec.scale);
        // SAFETY: as above.
        unsafe {
            (wpe.wpe_view_map)(view);
            (wpe.wpe_view_set_visible)(view, GTRUE);
        }
        super::policy::connect(page);
        page.mute(spec.muted);
        page.load(&spec.address);
        Ok(NonNull::from(page))
    }

    fn load(&self, address: &str) {
        let address = c_string(address);
        let wpe = self.wpe;
        // SAFETY: a request WebKit made, its headers, and the page's view.
        unsafe {
            let request = (wpe.webkit_uri_request_new)(address.as_ptr());
            let headers = (wpe.webkit_uri_request_get_http_headers)(request);
            if !headers.is_null() {
                (wpe.soup_message_headers_replace)(headers, c"Referer".as_ptr(), REFERER.as_ptr());
            }
            (wpe.webkit_web_view_load_request)(self.web_view, request);
            gobject_sys::g_object_unref(request.cast());
        }
    }

    /// The box is `size_pt` points at `scale` pixels a point.
    pub(super) fn resize(&self, size_pt: [f32; 2], scale: f32) {
        let side = |points: f32| {
            #[expect(clippy::cast_possible_truncation, reason = "a box side in points")]
            let side = points.round().max(1.0) as i32;
            side
        };
        // SAFETY: the page's toplevel; Papo's toplevel resizes its view.
        unsafe {
            (self.wpe.wpe_toplevel_scale_changed)(self.toplevel, f64::from(scale.max(0.5)));
            (self.wpe.wpe_toplevel_resize)(self.toplevel, side(size_pt[0]), side(size_pt[1]));
        }
    }

    /// Mutes or unmutes the page's sound (one sound at a time).
    pub(super) fn mute(&self, muted: bool) {
        let muted = if muted { GTRUE } else { GFALSE };
        // SAFETY: the page's view.
        unsafe { (self.wpe.webkit_web_view_set_is_muted)(self.web_view, muted) };
    }

    /// The page has the keyboard, or no longer has it.
    pub(super) fn focus(&self, focused: bool) {
        // SAFETY: the page's WPE view.
        unsafe {
            if focused {
                (self.wpe.wpe_view_focus_in)(self.view);
            } else {
                (self.wpe.wpe_view_focus_out)(self.view);
            }
        }
    }

    /// WebKit rendered `buffer`: it is shown, and the first frame says the player is up.
    pub(super) fn render(&self, buffer: Object) {
        match self.frames.render(self.view, buffer) {
            Ok(_) if !self.painted.replace(true) => {
                (self.events)(WebEvent::Page(self.id, PageEvent::Painted));
            }
            Ok(_) => {}
            Err(failure) => {
                log::warn!("a provider page's frame wasn't shown: {}", failure.0);
                (self.events)(WebEvent::Page(self.id, PageEvent::Failed));
            }
        }
    }

    /// WPE changed its buffers.
    pub(super) fn forget_buffers(&self) {
        self.frames.forget();
    }

    /// The page asked for the cursor `name` (CSS's names).
    pub(super) fn cursor(&self, name: &str) {
        (self.events)(WebEvent::Page(self.id, PageEvent::Cursor(name.to_owned())));
    }

    /// Closes the page: its frames stop, its handlers go, its objects are let go.
    pub(super) fn close(self: Box<Page>) {
        self.frames.close();
        classes::set_page(self.view, std::ptr::null());
        super::policy::disconnect(&self);
        // SAFETY: the page's own objects, released once; WebKit ends its processes' share.
        unsafe {
            (self.wpe.webkit_web_view_stop_loading)(self.web_view);
            (self.wpe.wpe_view_closed)(self.view);
            (self.wpe.wpe_toplevel_closed)(self.toplevel);
            gobject_sys::g_object_unref(self.toplevel.cast());
            gobject_sys::g_object_unref(self.web_view.cast());
        }
    }
}
