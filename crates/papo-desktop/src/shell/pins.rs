//! The pins panel from the shell's side (`docs/design.md` §10, Pins and
//! PanelPeek): 📌 or Ctrl+P opens it (fetching the pins again, as a pin of
//! a message never loaded arrives only with a fetch), a pin opens in the
//! conversation the way ⌛ opens its block, with `highlight.selected`, and
//! the panel stays open marking it. The pointer leaving it then shrinks it
//! to its header and that pin (`PanelPeek`): the pointer leaves before any
//! press on a message or scroll of the conversation, which never reach the
//! panel. 📌, Ctrl+P, the pointer coming back onto it or a click on it
//! stretch it back. With nothing opened, a press on the conversation or a
//! scroll of it under the pointer closes it (leaving it alone does not).
//! Escape closes it.
//! Unpin applies at once.

use std::time::Duration;

use papo_sdk::Uuid;
use papo_sdk_ui::{PinChange, message_actions};
use rune::Tx;

use super::Shell;
use crate::design::JUMP_HIGHLIGHT_S;
use crate::timeline::anchor::Anchor;
use crate::timeline::{TimelineKey, watch};

/// The element key of 📌, which the panel grows from.
pub(crate) const PINS_BUTTON_KEY: &str = "pins-button";
/// The element key of the open panel.
pub(crate) const PINS_PANEL_KEY: &str = "pins-panel";

/// The open pins panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PinsPanel {
    /// The conversation whose pins it shows; it closes when another shows.
    pub(crate) key: TimelineKey,
    /// The pin opened from it, which it marks.
    pub(crate) opened: Option<Uuid>,
    /// Shrunk to its header and the opened pin.
    pub(crate) peek: bool,
    /// The newest pin opening sent; only its answer counts.
    opening: u64,
}

impl Shell {
    /// 📌 or Ctrl+P: the panel opens, or a shrunk one stretches back, or
    /// an open one closes.
    pub(crate) fn pins_pressed(&mut self, tx: &mut Tx) {
        let Some(key) = self.tabs.shown().map(|open| open.key) else {
            return;
        };
        match &mut self.pins_panel {
            Some(panel) if panel.key == key && panel.peek => panel.peek = false,
            Some(panel) if panel.key == key => {
                self.close_pins(tx);
                return;
            }
            _ => {
                self.nav.menu = None;
                self.pins_panel = Some(PinsPanel {
                    key,
                    opened: None,
                    peek: false,
                    opening: 0,
                });
                self.refresh_pins(key);
            }
        }
        tx.focus(PINS_PANEL_KEY, true);
    }

    /// Escape, or 📌 on the open panel: it closes, and 📌 gets the
    /// keyboard.
    pub(crate) fn close_pins(&mut self, tx: &mut Tx) {
        if self.pins_panel.take().is_some() {
            tx.focus(PINS_BUTTON_KEY, true);
        }
    }

    /// A click outside the panel: it shrinks to the opened pin, or closes
    /// when none was opened.
    pub(crate) fn pins_outside(&mut self) {
        match &mut self.pins_panel {
            Some(panel) if panel.opened.is_some() => panel.peek = true,
            Some(_) => self.pins_panel = None,
            None => {}
        }
    }

    /// The conversation scrolled under the pointer: the panel shrinks to
    /// the opened pin, or closes with none opened, as for a press outside.
    /// Returns whether it changed.
    pub(crate) fn pins_scrolled(&mut self) -> bool {
        let Some(panel) = self.shown_pins_panel() else {
            return false;
        };
        if panel.peek {
            return false;
        }
        self.pins_outside();
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
    pub(crate) fn pins_hovered(&mut self) -> bool {
        match &mut self.pins_panel {
            Some(panel) if panel.peek => {
                panel.peek = false;
                true
            }
            _ => false,
        }
    }

    /// A pin was pressed (or Enter on it): it shows in the conversation, in
    /// place when the window holds it, else in a window opened at its exact
    /// place, with `highlight.selected`; the panel marks it.
    pub(crate) fn open_pin(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) {
        let Some(panel) = self.pins_panel.as_mut().filter(|panel| panel.key == key) else {
            return;
        };
        panel.opened = Some(id);
        panel.peek = false;
        panel.opening += 1;
        let opening = panel.opening;
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if open.holds(id) {
            self.jump_to_original(key, id, tx);
            return;
        }
        // A pin the window does not hold: there is no fetch by ID, so the
        // window opens around the pin's exact cursor.
        let Some(cursor) = open.pins.get(id).and_then(|pin| pin.message.cursor()) else {
            return;
        };
        let offset_pt = open.tracker.jump_offset_pt();
        let offsets = std::sync::Arc::clone(&self.local_time.offsets);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(key.account) else {
                return;
            };
            let config = watch::config(offsets);
            match papo_sdk_ui::Timeline::focused(session, key.channel, cursor, config).await {
                Ok(window) => updates.send_tx(move |shell, tx| {
                    shell.pin_opened(key, id, opening, window, offset_pt, tx)
                }),
                Err(error) => log::warn!("a pin did not open: {error}"),
            }
        });
    }

    /// The window opened around pin `id` (opening number `opening`): shown,
    /// if that pin is still the one the open panel last opened. Returns
    /// whether the view changed.
    fn pin_opened(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        opening: u64,
        window: papo_sdk_ui::Timeline,
        offset_pt: f32,
        tx: &mut Tx,
    ) -> bool {
        let current = self.shown_pins_panel().is_some_and(|panel| {
            panel.key == key && panel.opened == Some(id) && panel.opening == opening
        });
        if !current {
            return false;
        }
        let at = Anchor::Message { id, offset_pt };
        if let Some(opening) = self.tabs.adopt(key, window, at) {
            self.start_watch(opening);
        }
        if let Some(open) = self.shown_timeline_mut(key) {
            open.flash = Some((id, tx.clock() + JUMP_HIGHLIGHT_S));
        }
        tx.wake_after(Duration::from_secs_f64(JUMP_HIGHLIGHT_S));
        true
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

    /// Another conversation shows: what belonged to the one that showed
    /// (its pins panel, a stretched hover bar) closes.
    pub(crate) fn conversation_left(&mut self) {
        self.pins_panel = None;
        self.stretched = None;
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
    Strip,
}

impl Shell {
    /// What Escape would close now, if anything.
    pub(crate) fn on_top(&self) -> Option<OnTop> {
        if self.shown_pins_panel().is_some() {
            return Some(OnTop::PinsPanel);
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
            Some(OnTop::Strip) => self.stretched = None,
            None => return false,
        }
        true
    }
}
