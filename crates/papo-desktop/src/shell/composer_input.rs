//! The field's keys before it edits (`docs/design.md` §3, "Markdown in
//! place" and "Composer surfaces"), and the @ picker kept in step with the
//! field's text.
//!
//! The field asks here first. In order: the open picker's keys; Alt+↑/↓
//! to another channel (the field's editor would take them otherwise), not
//! while a composition is open; Tab past a closer; a delimiter or shortcut over a selection; Enter by the "Enter to
//! send" setting. Anything else is the field's own, and every key is marked
//! on its way: only a typed character or Backspace may close a pair.

use rune::{Key, KeyEvent, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::composer::Keystroke;
use crate::composer::picker::Picker;
use papo_sdk::MentionTarget;
use papo_sdk_ui::mention_suggestions;

impl Shell {
    /// The keys of replying and editing (`docs/design.md` §12), after the
    /// picker had its turn: Escape ends an edit (the field as it was comes
    /// back) or a reply (the text stays); in an empty field, ↑ edits your
    /// newest message and Ctrl+↑ goes into the conversation.
    fn mode_key(&mut self, key: &KeyEvent, bare: bool, tx: &mut Tx) -> bool {
        let empty = self
            .composer
            .shown()
            .is_some_and(|(_, draft)| draft.edit.text.is_empty());
        // ↑ edits only when the tray is empty too (§14.2): the attachments
        // would otherwise vanish under the edit.
        let tray_empty = self
            .composer
            .shown()
            .is_none_or(|(_, draft)| draft.tray.is_empty());
        let primary_only = (key.mods.cmd || key.mods.ctrl) && !key.mods.alt && !key.mods.shift;
        match key.key {
            Key::Escape if bare => {
                if self.escape_on_top() {
                    return true;
                }
                if let Some(effects) = self.composer.cancel_edit() {
                    self.run_composer(effects, Some(tx));
                    return true;
                }
                let Some(effects) = self.composer.cancel_reply() else {
                    return false;
                };
                self.run_composer(effects, Some(tx));
                true
            }
            Key::Up if bare && empty && tray_empty => self.edit_newest(tx),
            Key::Up if primary_only && empty => self.focus_newest_message(tx),
            _ => false,
        }
    }

    /// A key reached the field; whether it is taken here.
    pub(crate) fn composer_key(&mut self, key: &KeyEvent, enter_sends: bool, tx: &mut Tx) -> bool {
        // AltGr (Ctrl+Alt off macOS) and Option still type; a command
        // chord does not.
        let mods = key.mods;
        let typed = match &key.key {
            Key::Text(text) if !key.is_command_chord() => Some(Keystroke::Text(text.clone())),
            Key::Backspace if !mods.ctrl && !mods.cmd && !mods.alt && !mods.shift => {
                Some(Keystroke::Backspace)
            }
            _ => None,
        };
        self.composer.key_reached(tx.clock(), typed);
        let composing = self
            .composer
            .shown()
            .is_some_and(|(_, draft)| draft.edit.is_composing());
        if self.picker_key(key, tx) {
            return true;
        }
        // The editor would take Alt+arrows as word moves; these move between
        // channels instead (`docs/design.md` §12).
        let moved = !composing
            && (self.channel_step_key(key, tx)
                || self.unread_step_key(key, tx)
                || self.history_key(key, tx));
        if moved {
            return true;
        }
        let bare = !key.mods.ctrl && !key.mods.cmd && !key.mods.alt && !key.mods.shift;
        // Ctrl+Enter sends whatever Enter does (`docs/design.md` §12).
        let primary_alone = (mods.ctrl || mods.cmd) && !mods.alt && !mods.shift;
        if key.key == Key::Enter && primary_alone && !composing {
            self.composer_send(tx);
            return true;
        }
        if self.mode_key(key, bare, tx) {
            return true;
        }
        if key.key == Key::Tab && bare && self.composer.skip_pair() {
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
        false
    }

    /// The row of `target` was clicked.
    pub(crate) fn mention_picked(&mut self, target: MentionTarget, tx: &mut Tx) {
        if self.composer.pick(target) {
            self.composer_edited(tx);
        }
        tx.focus(COMPOSER_KEY, false);
    }

    /// The open picker's keys, with no modifier held: ↑ ↓ move, Enter or Tab
    /// picks, Escape closes.
    fn picker_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(chosen) = self.composer.picker().and_then(Picker::chosen_target) else {
            return false;
        };
        let mods = key.mods;
        if mods.ctrl || mods.cmd || mods.alt || mods.shift {
            return false;
        }
        match key.key {
            Key::Up | Key::Down => {
                let by = if key.key == Key::Up { -1 } else { 1 };
                self.composer.step_picker(by);
                if let Some(row) = self.composer.picker().and_then(Picker::chosen_key) {
                    tx.scroll_into_view(row);
                }
                true
            }
            Key::Enter | Key::Tab => {
                if self.composer.pick(chosen) {
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
        let found = mention_suggestions(&snapshot, key.account.user(), key.channel, &query.text);
        self.composer.show_picker(key, query, found);
    }
}
