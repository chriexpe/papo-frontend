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
use std::sync::Arc;

use glib_sys::{GFALSE, GTRUE};

use super::classes;
use super::frames::Frames;
use super::glib::{Context, Value, c_string, new_object};
use super::keys::{KeyPress, PageKey};
use super::library::{Object, Wpe};
use super::{Events, PageEvent, PageId, PageInput, PageSpec, PointerButton, WebEvent};

/// What `YouTube` and other players are told the browser is: a Chrome on Linux, as the
/// Chromium-based clients present themselves, so the player offers its desktop controls.
const USER_AGENT: &std::ffi::CStr = c"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
/// The page that embeds the player, as players ask (`YouTube` refuses to play without one):
/// Papo's own identity, never a real site.
const REFERER: &std::ffi::CStr = c"https://io.github.chriexpe.papo/";

const AUTOPLAY_ALLOW: i32 = 0;
const TOPLEVEL_ACTIVE: u32 = 1 << 2;
const EVENT_POINTER_DOWN: u32 = 1;
const EVENT_POINTER_UP: u32 = 2;
const EVENT_POINTER_MOVE: u32 = 3;
const EVENT_POINTER_LEAVE: u32 = 5;
const EVENT_KEY_DOWN: u32 = 6;
const EVENT_KEY_UP: u32 = 7;
const SOURCE_MOUSE: u32 = 0;
const SOURCE_KEYBOARD: u32 = 2;
const SOURCE_TOUCHPAD: u32 = 4;
const MODIFIER_BUTTON1: u32 = 1 << 8;

/// A provider page.
pub(super) struct Page {
    pub(super) id: PageId,
    pub(super) wpe: &'static Wpe,
    pub(super) web_view: Object,
    view: Object,
    toplevel: Object,
    keymap: Object,
    frames: Frames,
    /// Whether an address is still this player (the SDK's `ProviderPlayer::keeps`).
    pub(super) keeps: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub(super) events: Events,
    /// The player's document has started to load: later failures (a navigation the policy
    /// refused) leave it as it is.
    pub(super) committed: Cell<bool>,
    painted: Cell<bool>,
    /// WPE's bits for the pointer buttons held.
    buttons: Cell<u32>,
}

impl Page {
    /// Opens `spec`'s address on `display`, storing its site data in `session`.
    pub(super) fn open(
        wpe: &'static Wpe,
        context: Context,
        display: Object,
        session: Object,
        id: PageId,
        spec: PageSpec,
        events: Events,
    ) -> Result<Box<Page>, String> {
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
        let page = Box::new(Page {
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
            painted: Cell::new(false),
            buttons: Cell::new(0),
        });
        classes::set_page(view, &raw const *page);
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
        super::policy::connect(&page);
        page.mute(spec.muted);
        page.load(&spec.address);
        Ok(page)
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

    /// Forwards `input` to the page.
    pub(super) fn input(&self, input: PageInput) {
        let wpe = self.wpe;
        let time = now_ms();
        // SAFETY: WPE's event constructors for the page's view; each event is released after
        // the view took it.
        unsafe {
            let event = match input {
                PageInput::Move { x, y } => (wpe.wpe_event_pointer_move_new)(
                    EVENT_POINTER_MOVE,
                    self.view,
                    SOURCE_MOUSE,
                    time,
                    self.buttons.get(),
                    f64::from(x),
                    f64::from(y),
                    0.0,
                    0.0,
                ),
                PageInput::Leave => (wpe.wpe_event_pointer_move_new)(
                    EVENT_POINTER_LEAVE,
                    self.view,
                    SOURCE_MOUSE,
                    time,
                    self.buttons.get(),
                    -1.0,
                    -1.0,
                    0.0,
                    0.0,
                ),
                PageInput::Down { x, y, button } => {
                    let (number, bit) = button_bits(button);
                    let count = (wpe.wpe_view_compute_press_count)(
                        self.view,
                        f64::from(x),
                        f64::from(y),
                        number,
                        time,
                    );
                    self.buttons.set(self.buttons.get() | bit);
                    (wpe.wpe_event_pointer_button_new)(
                        EVENT_POINTER_DOWN,
                        self.view,
                        SOURCE_MOUSE,
                        time,
                        self.buttons.get(),
                        number,
                        f64::from(x),
                        f64::from(y),
                        count,
                    )
                }
                PageInput::Up { x, y, button } => {
                    let (number, bit) = button_bits(button);
                    self.buttons.set(self.buttons.get() & !bit);
                    // WPE takes a press count only on a press: anything else on a release
                    // makes no event.
                    (wpe.wpe_event_pointer_button_new)(
                        EVENT_POINTER_UP,
                        self.view,
                        SOURCE_MOUSE,
                        time,
                        self.buttons.get(),
                        number,
                        f64::from(x),
                        f64::from(y),
                        0,
                    )
                }
                PageInput::Wheel {
                    x,
                    y,
                    dx,
                    dy,
                    precise,
                } => (wpe.wpe_event_scroll_new)(
                    self.view,
                    if precise {
                        SOURCE_TOUCHPAD
                    } else {
                        SOURCE_MOUSE
                    },
                    time,
                    0,
                    f64::from(dx),
                    f64::from(dy),
                    if precise { GTRUE } else { GFALSE },
                    GFALSE,
                    f64::from(x),
                    f64::from(y),
                ),
                PageInput::Key(press) => {
                    self.key(press, time);
                    return;
                }
            };
            if !event.is_null() {
                (wpe.wpe_view_event)(self.view, event);
                (wpe.wpe_event_unref)(event);
            }
        }
    }

    /// A key goes down and comes up again: Rune reports presses only.
    fn key(&self, press: KeyPress, time: u32) {
        let wpe = self.wpe;
        // SAFETY: WPE's keysym lookup and event constructors for the page's view.
        unsafe {
            let keyval = match press.key {
                PageKey::Keysym(keysym) => keysym,
                PageKey::Char(character) => (wpe.wpe_unicode_to_keyval)(u32::from(character)),
            };
            let keycode = self.keycode(keyval);
            for kind in [EVENT_KEY_DOWN, EVENT_KEY_UP] {
                let event = (wpe.wpe_event_keyboard_new)(
                    kind,
                    self.view,
                    SOURCE_KEYBOARD,
                    time,
                    press.modifiers,
                    keycode,
                    keyval,
                );
                if !event.is_null() {
                    (wpe.wpe_view_event)(self.view, event);
                    (wpe.wpe_event_unref)(event);
                }
            }
        }
    }

    /// The hardware keycode the keymap puts `keyval` on, so the page's `code` is right; 0
    /// when the keymap has none.
    fn keycode(&self, keyval: u32) -> u32 {
        if self.keymap.is_null() {
            return 0;
        }
        let mut entries: *mut super::library::KeymapEntry = std::ptr::null_mut();
        let mut count: u32 = 0;
        // SAFETY: the keymap fills an array it allocates, freed here.
        unsafe {
            let found = (self.wpe.wpe_keymap_get_entries_for_keyval)(
                self.keymap,
                keyval,
                &raw mut entries,
                &raw mut count,
            );
            let keycode = if found != GFALSE && count > 0 && !entries.is_null() {
                (*entries).keycode
            } else {
                0
            };
            glib_sys::g_free(entries.cast());
            keycode
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

/// WPE's number for `button` and its modifier bit.
fn button_bits(button: PointerButton) -> (u32, u32) {
    match button {
        PointerButton::Primary => (1, MODIFIER_BUTTON1),
    }
}

/// A millisecond clock for event times.
fn now_ms() -> u32 {
    // SAFETY: a plain clock read.
    let micros = unsafe { glib_sys::g_get_monotonic_time() };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "event times wrap"
    )]
    let millis = (micros / 1000) as u32;
    millis
}
