//! The shell's side of message content: what a press inside a message does
//! (open a link or ask first, reveal a spoiler), a code block's "Copied",
//! and Ctrl+A, which selects one message rather than the whole timeline.
//!
//! The content's look is `timeline::content`'s; the link policy and its
//! confirmation are `links`'.

use std::time::Duration;

use papo_sdk::Uuid;
use rune::{KeyEvent, Tx};

use super::Shell;
use crate::design::COPIED_FOR_S;
use crate::links::{self, Decision, Prompt, Trusted};
use crate::timeline::content::Press;
use crate::timeline::content::copy::message_of;
use crate::timeline::{TimelineKey, rows};

/// What the shell keeps for message content.
#[derive(Default)]
pub(crate) struct Reading {
    /// The link waiting for the reader's answer.
    pub(crate) link: Option<Prompt>,
    pub(crate) trusted: Trusted,
    /// The message under the pointer: Ctrl+A selects it when no text is
    /// selected.
    hovered: Option<Uuid>,
}

impl Reading {
    /// Reads this device's trusted domains.
    pub(crate) fn load(&mut self) {
        self.trusted = Trusted::load();
    }
}

impl Shell {
    /// A press inside message content.
    pub(crate) fn content_pressed_in(
        &mut self,
        key: TimelineKey,
        message: Uuid,
        press: Press,
        tx: &mut Tx,
    ) {
        if !self.tabs.is_shown(key) {
            return;
        }
        match press {
            Press::Url(url) => self.link_pressed(key, url, tx),
            Press::Spoiler(index) => {
                if let Some(open) = self.tabs.shown_mut() {
                    open.reveal(message, index);
                }
            }
            Press::Edited => {}
        }
    }

    fn link_pressed(&mut self, key: TimelineKey, url: String, tx: &mut Tx) {
        let server_host = self
            .nav
            .servers()
            .get(key.account.server())
            .and_then(|server| server.host.as_deref());
        match links::decide(&url, server_host, &self.reading.trusted) {
            Decision::Open => open(&url, tx),
            Decision::Ask { host } => {
                self.reading.link = Some(Prompt {
                    url,
                    host,
                    always: false,
                });
            }
            Decision::Refuse => {}
        }
    }

    /// The confirmation's Cancel, or Escape.
    pub(crate) fn cancel_link(&mut self) {
        self.reading.link = None;
    }

    /// The confirmation's Open: opens the link, and trusts its host when
    /// the box is ticked.
    pub(crate) fn confirm_link(&mut self, tx: &mut Tx) {
        let Some(prompt) = self.reading.link.take() else {
            return;
        };
        if prompt.always {
            self.reading.trusted.add(prompt.host);
        }
        open(&prompt.url, tx);
    }

    /// The confirmation's "Always open links from {host}" box.
    pub(crate) fn toggle_trust_link(&mut self) {
        if let Some(prompt) = &mut self.reading.link {
            prompt.always = !prompt.always;
        }
    }

    /// Code block `index` of `message` was copied: its Copy reads "Copied"
    /// for a moment.
    pub(crate) fn code_copied_in(
        &mut self,
        key: TimelineKey,
        message: Uuid,
        index: usize,
        tx: &mut Tx,
    ) {
        let until_s = tx.clock() + COPIED_FOR_S;
        if let Some(open) = self.tabs.shown_mut().filter(|open| open.key == key) {
            open.code_copied(message, index, until_s);
            tx.wake_after(Duration::from_secs_f64(COPIED_FOR_S));
        }
    }

    /// Ends a "Copied" whose moment is over. Returns whether one ended.
    pub(crate) fn end_copied(&mut self, clock_s: f64) -> bool {
        self.tabs
            .shown_mut()
            .is_some_and(|open| open.end_copied(clock_s))
    }

    /// The pointer entered or left `message`.
    pub(crate) fn message_hovered_in(&mut self, key: TimelineKey, message: Uuid, hovered: bool) {
        if !self.tabs.is_shown(key) {
            return;
        }
        if hovered {
            self.reading.hovered = Some(message);
        } else if self.reading.hovered == Some(message) {
            self.reading.hovered = None;
        }
    }

    /// Keys no control took. Ctrl+A in the timeline selects one message:
    /// the one holding the selection, else the one under the pointer; never
    /// the whole timeline. A selection elsewhere keeps its own Ctrl+A.
    pub(crate) fn on_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let plain = !key.mods.shift && !key.mods.alt;
        if !key.is_command('a')
            || !plain
            || self.tabs.shown().is_none()
            || self.reading.link.is_some()
        {
            return false;
        }
        let message = match tx.text_selection() {
            Some(selection) => {
                let held = selection
                    .texts()
                    .iter()
                    .filter(|text| text.is_selected())
                    .find_map(|text| message_of(text.path()));
                let Some(message) = held else {
                    return false;
                };
                message
            }
            // Nothing under the pointer: the key is spent rather than
            // selecting every loaded message.
            None => match self.reading.hovered {
                Some(message) => message,
                None => return true,
            },
        };
        tx.select_all_text(rows::anchor_key(message));
        true
    }
}

/// Opens `url` in the system's handler.
fn open(url: &str, tx: &mut Tx) {
    if let Err(error) = tx.open_url(url) {
        log::warn!("a link did not open: {error}");
    }
}
