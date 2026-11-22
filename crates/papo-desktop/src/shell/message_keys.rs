//! A focused message's keys (`docs/design.md` §12): R replies, E edits
//! yours, Delete asks to delete, Escape goes back to the composer. A key
//! with a modifier passes on to the window (Ctrl+A on a focused message,
//! selecting it, comes with D5's message keys). Also Ctrl+↑ from
//! an empty composer into the conversation, and a reply line's jump to its
//! original, which then wears `highlight.selected` for 1.5 s.

use std::time::Duration;

use papo_sdk::Uuid;
use rune::{Key, KeyEvent, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::design::JUMP_HIGHLIGHT_S;
use crate::timeline::TimelineKey;
use crate::timeline::rows::{anchor_key, message_key};

impl Shell {
    /// A key on the focused message `id` of timeline `key`. Returns whether
    /// it was taken.
    pub(super) fn message_key(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        event: &KeyEvent,
        tx: &mut Tx,
    ) -> bool {
        let mods = event.mods;
        if mods.cmd || mods.ctrl || mods.alt || mods.logo {
            return false;
        }
        match &event.key {
            Key::Text(letter) if letter.eq_ignore_ascii_case("r") => self.reply_to(key, id, tx),
            Key::Text(letter) if letter.eq_ignore_ascii_case("e") => self.edit(key, id, tx),
            Key::Delete => {
                self.ask_delete(key, id);
                true
            }
            Key::Escape => {
                tx.focus(COMPOSER_KEY, false);
                true
            }
            _ => false,
        }
    }

    /// Ctrl+↑ in an empty composer: the newest message in the conversation
    /// takes the keyboard. Returns whether one did.
    pub(super) fn focus_newest_message(&mut self, tx: &mut Tx) -> bool {
        let Some((key, _)) = self.composer.shown() else {
            return false;
        };
        let newest = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| {
                open.items
                    .iter()
                    .rev()
                    .find_map(|item| match item.as_ref() {
                        papo_sdk_ui::TimelineItem::Message { id, .. } => Some(*id),
                        _ => None,
                    })
            });
        let Some(id) = newest else {
            return false;
        };
        tx.scroll_into_view(anchor_key(id));
        // Focus names an element's own key, the row's; it lands on the
        // focusable message inside.
        tx.focus(message_key(id), true);
        true
    }

    /// A reply line was pressed: its original shows (in place, or in a
    /// window opened around it) and wears `highlight.selected` for a moment.
    pub(super) fn jump_to_original(&mut self, key: TimelineKey, original: Uuid, tx: &mut Tx) {
        let until_s = tx.clock() + JUMP_HIGHLIGHT_S;
        if let Some(open) = self.shown_timeline_mut(key) {
            open.flash = Some((original, until_s));
        }
        tx.wake_after(Duration::from_secs_f64(JUMP_HIGHLIGHT_S));
        self.jump_to(key, original, tx);
    }

    /// Ends a jump's highlight once its time is over. Returns whether the
    /// view changed.
    pub(super) fn poll_flash(&mut self, clock_s: f64) -> bool {
        let Some(open) = self.tabs.shown_mut() else {
            return false;
        };
        match open.flash {
            Some((_, until_s)) if until_s <= clock_s => {
                open.flash = None;
                true
            }
            _ => false,
        }
    }
}
