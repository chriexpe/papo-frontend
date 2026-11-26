//! The input Papo forwards to a page, as WPE events on its view: the pointer (moves, primary
//! presses with their counts, leaving), the wheel (in the viewer), and keys (each pressed and
//! released, with the keymap's keycode so the page's `code` is right). Which input reaches a
//! page at all is the player's (`link_cards::player`, `shell::player_presses`).
//!
//! Safety: on the WebView thread, with the page's live view; every event is released after
//! the view took it.

#![expect(unsafe_code, reason = "WPE's input event constructors")]

use glib_sys::{GFALSE, GTRUE};

use super::Page;
use crate::web_view::keys::{KeyPress, PageKey};
use crate::web_view::{PageInput, PointerButton};

const EVENT_POINTER_DOWN: u32 = 1;
const EVENT_POINTER_UP: u32 = 2;
const EVENT_POINTER_MOVE: u32 = 3;
const EVENT_POINTER_LEAVE: u32 = 5;
// 6 is WPE_EVENT_SCROLL.
const EVENT_KEY_DOWN: u32 = 7;
const EVENT_KEY_UP: u32 = 8;
const SOURCE_MOUSE: u32 = 0;
const SOURCE_KEYBOARD: u32 = 2;
const SOURCE_TOUCHPAD: u32 = 4;
const MODIFIER_BUTTON1: u32 = 1 << 8;

impl Page {
    /// Forwards `input` to the page.
    pub(in crate::web_view) fn input(&self, input: PageInput) {
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
        let mut entries: *mut crate::web_view::library::KeymapEntry = std::ptr::null_mut();
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
