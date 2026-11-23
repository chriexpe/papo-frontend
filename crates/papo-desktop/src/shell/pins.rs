//! The pins panel from the shell's side (`docs/design.md` §10, Pins and
//! PanelPeek): 📌 or Ctrl+P opens it (fetching the pins again, as a pin of
//! a message never loaded arrives only with a fetch), a pin opens in the
//! conversation the way ⌛ opens its block, with `highlight.selected`, and
//! the panel stays open marking it. A click outside then shrinks it to its
//! header and that pin (`PanelPeek`); 📌, Ctrl+P or a click on it stretch it
//! back. With nothing opened, a click outside closes it. Escape closes it.
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

    /// The panel, when it is open on the shown conversation.
    pub(crate) fn shown_pins_panel(&self) -> Option<PinsPanel> {
        let shown = self.tabs.shown()?.key;
        self.pins_panel.filter(|panel| panel.key == shown)
    }

    /// Fetches timeline `key`'s pins again.
    fn refresh_pins(&self, key: TimelineKey) {
        let Some(handle) = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| open.pins_handle.clone())
        else {
            return;
        };
        if let Some(sdk) = &self.sdk {
            sdk.run(move |_client| async move { watch::refresh(Some(handle)) });
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
        let Some(open) = self.shown_timeline_mut(key) else {
            return;
        };
        if open.holds(id) {
            self.jump_to_original(key, id, tx);
            return;
        }
        // A pin the window does not hold: there is no fetch by ID, so the
        // window opens around the pin's exact cursor.
        let Some(cursor) = open
            .pins
            .iter()
            .flatten()
            .find(|pin| pin.id.as_deref() == Some(id.to_string().as_str()))
            .and_then(|pin| pin.cursor())
        else {
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
                    let at = Anchor::Message { id, offset_pt };
                    if let Some(opening) = shell.tabs.adopt(key, window, at) {
                        shell.start_watch(opening);
                    }
                    if let Some(open) = shell.shown_timeline_mut(key) {
                        open.flash = Some((id, tx.clock() + JUMP_HIGHLIGHT_S));
                    }
                    tx.wake_after(Duration::from_secs_f64(JUMP_HIGHLIGHT_S));
                    true
                }),
                Err(error) => log::warn!("a pin did not open: {error}"),
            }
        });
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
        let Some(pin) = open
            .pins
            .iter()
            .flatten()
            .find(|pin| pin.id.as_deref() == Some(id.to_string().as_str()))
        else {
            return false;
        };
        let capabilities = super::replies::channel_capabilities(open);
        message_actions(pin, None, Some(true), key.account.user(), capabilities).pin
            == Some(PinChange::Unpin)
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
        self.stretched.map(|_| OnTop::Strip)
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
