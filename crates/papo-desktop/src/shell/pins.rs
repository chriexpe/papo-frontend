//! The pins panel from the shell's side (`docs/design.md` §10, Pins and
//! `PanelPeek`): 📌 or Ctrl+P opens it (fetching the pins again, as a pin of
//! a message never loaded arrives only with a fetch), a pin opens in the
//! conversation (`message_open`), and the panel stays open marking it.
//!
//! One rule decides the rest, and search shares it: input that starts
//! outside the panel (a press anywhere else in the window, a wheel over the
//! conversation, the channels or the rail, typing, a key on a message)
//! means you are back in the conversation (`worked_outside`). The panel
//! then shrinks to its header and the opened pin, or closes when nothing
//! was opened from it. Input on it (the pointer coming onto it, a press,
//! 📌 or Ctrl+P) stretches it back, the opened pin in view. The app acting
//! on its own (messages arriving, a jump, a resize) never counts. Once a
//! pin is opened, the pointer leaving the panel shrinks it early
//! (Christian, 2026-11-23). Escape closes it, giving the keyboard back to
//! where it was. The search panel (`search`) follows the same rule; one
//! header panel is open at a time.

use papo_sdk::Uuid;
use papo_sdk_ui::{PinChange, message_actions};
use rune::Tx;

use super::Shell;
use crate::timeline::{TimelineKey, watch};

/// The element key of 📌, which the panel grows from.
pub(crate) const PINS_BUTTON_KEY: &str = "pins-button";
/// The element key of the open panel.
pub(crate) const PINS_PANEL_KEY: &str = "pins-panel";

/// The element key of pin `id`'s row in the panel.
pub(crate) fn pin_row_key(id: Uuid) -> String {
    format!("pin-row-{id}")
}

/// The open pins panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PinsPanel {
    /// The conversation whose pins it shows; it closes when another shows.
    pub(crate) key: TimelineKey,
    /// The pin opened from it, which it marks.
    pub(crate) opened: Option<Uuid>,
    /// Shrunk to its header and the opened pin.
    pub(crate) peek: bool,
}

impl Shell {
    /// 📌 or Ctrl+P (`keyboard`): the panel opens, or a shrunk one stretches
    /// back, or an open one closes.
    pub(crate) fn pins_pressed(&mut self, keyboard: bool, tx: &mut Tx) {
        let Some(key) = self.tabs.shown().map(|open| open.key) else {
            return;
        };
        match self.pins_panel {
            Some(panel) if panel.key == key && panel.peek => {
                self.stretch_pins(tx);
                return;
            }
            Some(panel) if panel.key == key => {
                self.close_pins(tx);
                return;
            }
            _ => {}
        }
        self.nav.menu = None;
        // One header panel at a time.
        self.fold_search();
        self.pins_panel = Some(PinsPanel {
            key,
            opened: None,
            peek: false,
        });
        self.pins_return_focus = tx.focused().map(str::to_owned);
        self.refresh_pins(key);
        // The keyboard follows a keyboard open; a click moves no focus.
        if keyboard {
            tx.focus(PINS_PANEL_KEY, true);
        }
    }

    /// Escape, or 📌 on the open panel: it closes, and what had the
    /// keyboard before it opened gets it back.
    pub(crate) fn close_pins(&mut self, tx: &mut Tx) {
        if self.pins_panel.take().is_some() {
            self.cancel_opening();
            match self.pins_return_focus.take() {
                Some(key) => tx.focus(key, true),
                None => tx.focus(PINS_BUTTON_KEY, true),
            }
        }
    }

    /// Input that started outside the panel (a press elsewhere, a wheel
    /// over the conversation, the channels or the rail, typing, a key on a
    /// message): the panel shrinks to the opened pin, or closes when
    /// nothing was opened. The one rule every outside path calls. Returns
    /// whether it changed.
    pub(crate) fn worked_outside(&mut self) -> bool {
        let search = self.search_worked_outside();
        self.pins_worked_outside() || search
    }

    fn pins_worked_outside(&mut self) -> bool {
        let Some(panel) = self.shown_pins_panel() else {
            return false;
        };
        match panel.opened {
            Some(_) if panel.peek => false,
            Some(_) => {
                if let Some(panel) = &mut self.pins_panel {
                    panel.peek = true;
                }
                true
            }
            None => {
                self.pins_panel = None;
                true
            }
        }
    }

    /// A shrunk panel stretches back, its opened pin brought into view, so
    /// what is under the pointer stays the pin it pointed at.
    fn stretch_pins(&mut self, tx: &mut Tx) -> bool {
        let Some(panel) = self.pins_panel.as_mut().filter(|panel| panel.peek) else {
            return false;
        };
        panel.peek = false;
        if let Some(id) = panel.opened {
            tx.scroll_into_view(pin_row_key(id));
        }
        true
    }

    /// The pointer left the panel: once a pin was opened from it, the
    /// panel shrinks to that pin (the conversation is where you work now).
    pub(crate) fn pins_left(&mut self) {
        if let Some(panel) = self
            .pins_panel
            .as_mut()
            .filter(|panel| panel.opened.is_some())
        {
            panel.peek = true;
        }
    }

    /// The panel, when it is open on the shown conversation.
    pub(crate) fn shown_pins_panel(&self) -> Option<PinsPanel> {
        let shown = self.tabs.shown()?.key;
        self.pins_panel.filter(|panel| panel.key == shown)
    }

    /// Fetches timeline `key`'s pins again; a failure is said in the panel.
    fn refresh_pins(&mut self, key: TimelineKey) {
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        open.pins.failed = false;
        let Some(handle) = open.pins_handle.clone() else {
            return;
        };
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |_client| async move {
            watch::refresh(handle, move || {
                updates.send(move |shell| {
                    let Some(open) = shell.shown_timeline_mut(key) else {
                        return false;
                    };
                    open.pins.failed = true;
                    true
                });
            });
        });
    }

    /// The pointer came onto the panel: a shrunk one stretches back (a
    /// scroll over it comes this way too).
    pub(crate) fn pins_hovered(&mut self, tx: &mut Tx) -> bool {
        self.stretch_pins(tx)
    }

    /// A pin was pressed, or Enter on it (`keyboard`): it shows in the
    /// conversation (`open_message`) and the panel marks it. From the
    /// keyboard, the message takes the keyboard (the conversation is where
    /// you work now) and the panel shrinks to it.
    pub(crate) fn open_pin(&mut self, key: TimelineKey, id: Uuid, keyboard: bool, tx: &mut Tx) {
        let Some(panel) = self.pins_panel.as_mut().filter(|panel| panel.key == key) else {
            return;
        };
        panel.opened = Some(id);
        panel.peek = false;
        let cursor = self
            .shown_timeline(key)
            .and_then(|open| open.pins.get(id))
            .and_then(|pin| pin.message.cursor());
        self.open_message(key, id, cursor, keyboard, tx);
    }

    /// Unpin on a pin of the panel: it applies at once, if the account may.
    pub(crate) fn unpin_from_panel(&mut self, key: TimelineKey, id: Uuid) {
        if self.may_unpin(key, id) {
            self.send_pin(key, id, false);
        }
    }

    /// Whether the account may unpin pin `id` of the shown conversation.
    pub(crate) fn may_unpin(&self, key: TimelineKey, id: Uuid) -> bool {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        let Some(pin) = open.pins.get(id) else {
            return false;
        };
        let capabilities = super::replies::channel_capabilities(open);
        message_actions(
            &pin.message,
            None,
            Some(true),
            key.account.user(),
            capabilities,
        )
        .pin == Some(PinChange::Unpin)
    }

    /// The shown timeline's pins changed: an opened pin no longer among
    /// them is forgotten, so the panel never shrinks to nothing.
    pub(crate) fn pins_changed(&mut self, key: TimelineKey) {
        let Some(open) = self.tabs.open_mut(key) else {
            return;
        };
        let Some(panel) = self.pins_panel.as_mut().filter(|panel| panel.key == key) else {
            return;
        };
        let gone = panel
            .opened
            .is_some_and(|id| open.pins.pinned(id) == Some(false));
        if gone {
            panel.opened = None;
            panel.peek = false;
        }
    }

    /// Another conversation shows: what belonged to the one that showed
    /// (its pins panel, a stretched hover bar) closes.
    pub(crate) fn conversation_left(&mut self) {
        self.pins_panel = None;
        self.stretched = None;
        self.search_conversation_left();
    }
}

/// What Escape closes, top first (`docs/design.md` §12, Close what is on
/// top). Menus and dialogs take it themselves, as modals; under them, the
/// pins panel, then a hover bar stretched into its strip. Only then does
/// Escape do its own work where the keyboard is (end a reply or an edit,
/// leave a message for the composer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnTop {
    PinsPanel,
    Search,
    Strip,
}

impl Shell {
    /// What Escape would close now, if anything.
    pub(crate) fn on_top(&self) -> Option<OnTop> {
        if self.shown_pins_panel().is_some() {
            return Some(OnTop::PinsPanel);
        }
        if self.shown_search().is_some() {
            return Some(OnTop::Search);
        }
        let shown = self.tabs.shown()?.key;
        self.stretched
            .filter(|stretched| stretched.key == shown)
            .map(|_| OnTop::Strip)
    }

    /// Escape where the keyboard is: closes what is on top, keeping the
    /// keyboard there. Returns whether it closed anything.
    pub(crate) fn escape_on_top(&mut self) -> bool {
        match self.on_top() {
            Some(OnTop::PinsPanel) => self.pins_panel = None,
            Some(OnTop::Search) => self.fold_search(),
            Some(OnTop::Strip) => self.stretched = None,
            None => return false,
        }
        true
    }
}
