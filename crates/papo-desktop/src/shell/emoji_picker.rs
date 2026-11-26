//! The emoji picker from the shell's side (`docs/design.md` §16.1, §16.2, §16.5): opening it on
//! the Emoji tab for the field (the pill's 😊, Ctrl+E) or for a reaction (More emoji), its
//! search and keys, and what a pick does: it goes in at the field's caret, or reacts.
//!
//! The list and the cursor are `emoji_picker`'s; drawing is `emoji_picker::view`'s, with what it
//! reads gathered here. The picker is modal: a press outside closes it, its own keys come here
//! first, and the app's shortcuts wait while it is open.

use std::ops::Range;
use std::sync::Arc;

use papo_sdk::{AccountId, Uuid};
use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};

use super::Shell;
use crate::composer::{COMPOSER_KEY, COMPOSER_PILL_KEY};
use crate::design::Look;
use crate::emoji_picker::view::{EMOJI_SEARCH_KEY, PickerLook};
use crate::emoji_picker::{EmojiChoice, EmojiPicker, PickerFor, Section, Step};
use crate::picker::OpenPicker;
use crate::timeline::TimelineKey;

impl Shell {
    /// Whether the picker is open for the shown field: the pill's 😊 is lit.
    pub(crate) fn emoji_open_for_field(&self) -> bool {
        self.emoji_picker()
            .is_some_and(|picker| matches!(picker.target, PickerFor::Field(_)))
    }

    /// The pill's 😊, or Ctrl+E: the Emoji tab opens for the shown field, or closes when it is
    /// open for it.
    pub(crate) fn emoji_button_pressed(&mut self, tx: &mut Tx) {
        if self.emoji_open_for_field() {
            self.close_emoji_picker(tx);
            return;
        }
        // Not while the draft is read: a pick would have nowhere to go.
        let Some((key, _)) = self
            .composer
            .shown()
            .filter(|_| !self.composer.is_loading())
        else {
            return;
        };
        // While an edit is under way the field is the edit's: a pick goes in there too.
        self.open_emoji_picker(PickerFor::Field(key), tx);
    }

    /// More emoji on message `id`'s strip (the bar's or the menu's): the Emoji tab opens alone,
    /// and a pick reacts.
    pub(crate) fn more_emoji_pressed(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) {
        if !self.may_react(key, id) {
            return;
        }
        self.open_emoji_picker(PickerFor::Reaction { key, message: id }, tx);
    }

    /// Opens the Emoji tab for `target`, from the account's catalog and its emoji count as they
    /// stand now; the keyboard goes to its search.
    fn open_emoji_picker(&mut self, target: PickerFor, tx: &mut Tx) {
        let account = target.key().account;
        let usage = self
            .activity
            .get(&account)
            .map_or_else(|| Arc::from([]), |activity| Arc::clone(&activity.usage));
        let catalog = self.catalog(account).cloned();
        // It stands where the menu, a panel or a GIF tab on its way would: they close.
        self.gif.cancel_opening();
        self.worked_outside();
        self.nav.menu = None;
        self.stretched = None;
        self.picker = Some(OpenPicker::Emoji(EmojiPicker::new(
            target,
            catalog.as_deref(),
            usage,
        )));
        tx.focus(EMOJI_SEARCH_KEY, true);
    }

    /// A picker for a reaction whose message went away (deleted, or its conversation no longer
    /// shown) closes; the keyboard goes back to the field. Returns whether it closed.
    pub(super) fn poll_emoji_picker(&mut self, tx: &mut Tx) -> bool {
        let Some(PickerFor::Reaction { key, message }) =
            self.emoji_picker().map(|picker| picker.target)
        else {
            return false;
        };
        let held = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .is_some_and(|open| open.holds(message));
        if held {
            return false;
        }
        self.picker = None;
        tx.focus(COMPOSER_KEY, false);
        true
    }

    /// The search field's text, for its `on_edit`. With no picker (an event that arrives as it
    /// closes) a spare that nothing reads.
    pub(crate) fn emoji_search_mut(&mut self) -> &mut TextEdit {
        if let Some(OpenPicker::Emoji(picker)) = &mut self.picker {
            &mut picker.search
        } else {
            self.picker_spare = TextEdit::new("");
            &mut self.picker_spare
        }
    }

    /// The search changed: the list is found again.
    pub(crate) fn emoji_searched(&mut self, _tx: &mut Tx) {
        let Some(account) = self
            .emoji_picker()
            .map(|picker| picker.target.key().account)
        else {
            return;
        };
        let catalog = self.catalog(account).cloned();
        if let Some(picker) = self.emoji_picker_mut() {
            picker.rebuild(catalog.as_deref());
        }
        // Other emoji may stand where the lists last said; theirs are asked for.
        self.want_emoji();
    }

    /// The search's ✕.
    pub(crate) fn emoji_search_cleared(&mut self, tx: &mut Tx) {
        if let Some(picker) = self.emoji_picker_mut() {
            picker.search = TextEdit::new("");
        }
        self.emoji_searched(tx);
        tx.focus(EMOJI_SEARCH_KEY, false);
    }

    /// The account's catalog changed while the picker lists it.
    pub(super) fn emoji_picker_catalog_changed(&mut self, account: AccountId) {
        let catalog = self.catalog(account).cloned();
        if let Some(picker) = self
            .emoji_picker_mut()
            .filter(|picker| picker.target.key().account == account)
        {
            picker.rebuild(catalog.as_deref());
        }
    }

    /// A key in the search (§16.5): ↓ or Tab into the results, the arrows and Home / End over
    /// them, Enter picks (Shift+Enter keeps the picker open), Escape clears the search first,
    /// then closes. Returns whether it was taken.
    pub(crate) fn emoji_picker_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if mods.ctrl || mods.cmd || mods.alt || mods.logo {
            return false;
        }
        let Some(picker) = self.emoji_picker_mut() else {
            return false;
        };
        let in_results = picker.cursor.is_some();
        let step = match key.key {
            Key::Escape if !mods.shift => {
                if picker.search.text.is_empty() {
                    self.close_emoji_picker(tx);
                } else {
                    self.emoji_search_cleared(tx);
                }
                return true;
            }
            Key::Enter => {
                let chosen = picker.chosen().cloned().or_else(|| {
                    // From the search, Enter takes a search's first match.
                    (!picker.query().is_empty())
                        .then(|| picker.cells().first().cloned())
                        .flatten()
                });
                if let Some(choice) = chosen {
                    self.emoji_picked(&choice, mods.shift, tx);
                }
                return true;
            }
            Key::Down if !mods.shift => Step::Down,
            Key::Tab if !mods.shift && !in_results => Step::Enter,
            // Tab on from the results goes where it would; the cell lets go of the keyboard.
            Key::Tab => {
                picker.cursor = None;
                return false;
            }
            Key::Up if !mods.shift && in_results => Step::Up,
            Key::Left if !mods.shift && in_results => Step::Left,
            Key::Right if !mods.shift && in_results => Step::Right,
            Key::Home if !mods.shift && in_results => Step::Home,
            Key::End if !mods.shift && in_results => Step::End,
            _ => return false,
        };
        picker.step(step);
        if let Some(row) = picker.chosen_row_path() {
            tx.scroll_into_view(row);
        }
        true
    }

    /// The pointer came onto cell `at` (`on`) or left it.
    pub(crate) fn emoji_hovered(&mut self, at: usize, on: bool) {
        if let Some(picker) = self.emoji_picker_mut() {
            if on {
                picker.hovered = Some(at);
            } else if picker.hovered == Some(at) {
                picker.hovered = None;
            }
        }
    }

    /// A press on cell `at`; Shift held (`keep`) keeps the picker open.
    pub(crate) fn emoji_cell_pressed(&mut self, at: usize, keep: bool, tx: &mut Tx) {
        let choice = self
            .emoji_picker()
            .and_then(|picker| picker.cells().get(at).cloned());
        if let Some(choice) = choice {
            self.emoji_picked(&choice, keep, tx);
        }
    }

    /// A pick (§16.2): for the field it goes in at the caret, and the picker closes unless
    /// `keep`; for a reaction it reacts and the picker closes. A reaction already yours stays
    /// (the picker adds; the chip takes back).
    fn emoji_picked(&mut self, choice: &EmojiChoice, keep: bool, tx: &mut Tx) {
        let Some(target) = self.emoji_picker().map(|picker| picker.target) else {
            return;
        };
        match target {
            PickerFor::Field(key) => {
                if self.composer.insert_emoji(key, choice) {
                    // The pick's own edit is not work outside the picker: it stays open for
                    // Shift.
                    let picker = self.picker.take();
                    self.composer_edited(tx);
                    self.picker = picker;
                }
                if keep {
                    tx.focus(EMOJI_SEARCH_KEY, false);
                } else {
                    self.close_emoji_picker(tx);
                }
            }
            PickerFor::Reaction { key, message } => {
                let emoji = choice.reaction();
                if self.reacts_with(key, message, &emoji) == Some(false) {
                    self.react(key, message, emoji, true);
                }
                self.close_emoji_picker(tx);
            }
        }
    }

    /// The list shows sections `range`; their rows on screen are asked for as they report.
    pub(crate) fn emoji_sections_shown(&mut self, range: Range<usize>) -> bool {
        let changed = self
            .emoji_picker_mut()
            .is_some_and(|picker| picker.sections_shown(range));
        if changed {
            self.want_emoji();
        }
        false
    }

    /// Section `section` shows its rows of cells `range`: their custom emoji are fetched.
    pub(crate) fn emoji_rows_shown(&mut self, section: Section, range: Range<usize>) -> bool {
        let changed = self
            .emoji_picker_mut()
            .is_some_and(|picker| picker.rows_shown(section, range));
        if changed {
            self.want_emoji();
        }
        false
    }
}

/// The Emoji tab drawn (§16.1, §16.2): grown from the text pill, or for a reaction from its
/// message's row, where the hover bar sits; as wide as the pill at most.
pub(super) fn view(shell: &Shell, picker: &EmojiPicker, look: &Look, cx: &Cx<Shell>) -> El {
    let account = picker.target.key().account;
    // A reaction's message row may be scrolled out of the list: the picker then grows from the
    // pill, so it never stands unseen holding the keyboard.
    let anchor = match picker.target {
        PickerFor::Reaction { message, .. } => Some(crate::timeline::rows::message_key(message))
            .filter(|row| cx.layout_size(row).is_some()),
        PickerFor::Field(_) => None,
    }
    .unwrap_or_else(|| COMPOSER_PILL_KEY.to_owned());
    let server = shell.server_name(account);
    let conversation_height_pt = shell.conversation_rect[3];
    let given = PickerLook {
        look,
        emoji: shell.emoji_images.looks(),
        account,
        server,
        anchor: &anchor,
        anchor_width_pt: cx.layout_size(COMPOSER_PILL_KEY).map(|[width, _]| width),
        conversation_height_pt: (conversation_height_pt > 0.0).then_some(conversation_height_pt),
        language: crate::strings::emoji_language(),
    };
    crate::emoji_picker::view::view(picker, &given)
}
