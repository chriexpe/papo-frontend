//! The field's keys before it edits (`docs/design.md` §3, "Markdown in
//! place" and "Composer surfaces"), and the @ picker kept in step with the
//! caret.
//!
//! The field asks here first. In order: the open picker's keys; Tab past a
//! closer; a delimiter or shortcut over a selection; Enter by the "Enter to
//! send" setting. Anything else is the field's own, and a typed character
//! or Backspace is marked as typing on its way, since only that may close
//! a pair.

use papo_sdk::permissions::global_permissions;
use rune::{Key, KeyEvent, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::composer::mentions::candidates;
use crate::composer::picker::row_key;

impl Shell {
    /// A key reached the field; whether it is taken here.
    pub(crate) fn composer_key(&mut self, key: &KeyEvent, enter_sends: bool, tx: &mut Tx) -> bool {
        if self.picker_key(key, tx) {
            return true;
        }
        let plain = !key.mods.ctrl && !key.mods.cmd && !key.mods.alt;
        if key.key == Key::Tab && plain && !key.mods.shift && self.composer.skip_pair() {
            self.composer_edited(tx);
            return true;
        }
        if let Some(format) = crate::composer::wrap_format(key) {
            let by_shortcut = matches!(key.key, Key::Command(_));
            if self.composer.wrap(&format, by_shortcut) {
                self.composer_edited(tx);
                return true;
            }
        }
        if key.key == Key::Enter && !enter_sends {
            // Off, the keys swap. Plain Enter is taken here, not on submit,
            // so a held Enter adds a line per repeat as typing does (Rune
            // submits once per press).
            if key.mods.shift {
                self.composer_send(tx);
            } else {
                self.composer_new_line(tx);
            }
            return true;
        }
        let typed = match &key.key {
            Key::Text(_) => plain,
            Key::Backspace => plain && !key.mods.shift,
            _ => false,
        };
        if typed {
            self.composer.key_typed(tx.clock());
        }
        false
    }

    /// The caret moved without an edit (a click, an arrow): the picker
    /// follows it.
    pub(crate) fn composer_moved(&mut self) {
        self.refresh_picker();
    }

    /// Row `index` of the picker was clicked.
    pub(crate) fn mention_picked(&mut self, index: usize, tx: &mut Tx) {
        if self.composer.pick(index) {
            self.composer_edited(tx);
        }
        tx.focus(COMPOSER_KEY, false);
    }

    /// The open picker's keys: ↑ ↓ move, Enter or Tab picks, Escape closes.
    fn picker_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(picker) = self.composer.picker() else {
            return false;
        };
        let selected = picker.selected();
        match key.key {
            Key::Up | Key::Down => {
                let by = if key.key == Key::Up { -1 } else { 1 };
                self.composer.step_picker(by);
                if let Some(picker) = self.composer.picker() {
                    tx.scroll_into_view(row_key(picker.selected()));
                }
                true
            }
            Key::Enter | Key::Tab => {
                if self.composer.pick(selected) {
                    self.composer_edited(tx);
                }
                true
            }
            Key::Escape => {
                self.composer.dismiss_picker();
                true
            }
            _ => false,
        }
    }

    /// Opens, narrows or closes the picker for the @ query at the shown
    /// field's caret, from the roster of the account it writes in.
    pub(super) fn refresh_picker(&mut self) {
        let Some((key, query)) = self.composer.mention_query() else {
            self.composer.close_picker();
            return;
        };
        let resources = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| open.resources.clone());
        let Some(snapshot) = resources else {
            self.composer.close_picker();
            return;
        };
        // @everyone only where the account may notify everyone; unknown
        // permissions leave it out.
        let everyone = global_permissions(&snapshot, key.account.user())
            .is_ok_and(|granted| granted.everyone_message);
        let found = candidates(&snapshot, &query.text, everyone);
        self.composer.show_picker(key, query, found);
    }
}
