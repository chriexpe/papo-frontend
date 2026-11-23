//! The search field's keys and picks (`docs/design.md` §10, Search, and
//! §12): ↑ and ↓ through the list under it, Enter or Tab to pick (a space
//! follows, as after a value typed out in full), Enter to search at once
//! without a list, ↓ into the results, Escape to fold; and Clear filters.
//! What the list holds is `search_field`'s.

use papo_sdk::FilterValue;
use rune::{KeyEvent, Tx};

use super::Shell;
use super::search::SEARCH_FIELD_KEY;
use super::search_field::{FieldList, keyword_text, written};

impl Shell {
    /// A key in the field: the list's ↑, ↓, Enter and Tab, Enter to search
    /// at once without one, Escape to fold. Returns whether it was taken.
    pub(crate) fn search_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if mods.ctrl || mods.alt || mods.cmd || mods.logo {
            return false;
        }
        let rows = self.search_list().as_ref().map_or(0, FieldList::key_rows);
        match key.key {
            rune::Key::Escape => {
                self.close_search(tx);
                true
            }
            rune::Key::Down | rune::Key::Up if rows > 0 => {
                let by = if key.key == rune::Key::Down { 1 } else { -1 };
                if let Some(search) = self.search.as_mut() {
                    search.highlight =
                        crate::composer::picker::wrapped_step(search.highlight, rows, by);
                }
                true
            }
            rune::Key::Enter | rune::Key::Tab if rows > 0 && !mods.shift => {
                let at = self.search.as_ref().map_or(0, |search| search.highlight);
                self.search_picked(at.min(rows - 1), tx);
                true
            }
            // Without a list, ↓ goes into the results (`docs/design.md`
            // §12: lists move with ↑ and ↓).
            rune::Key::Down if !mods.shift => self.focus_first_hit(tx),
            // A value typed out in full ends as a picked one does.
            rune::Key::Tab if !mods.shift => self.end_typed_value(tx),
            rune::Key::Enter => {
                self.end_typed_value(tx);
                self.search_now();
                true
            }
            _ => false,
        }
    }

    /// Row `at` of the list now was picked: a filter's keyword goes in,
    /// or a value replaces what was typed for it, followed by a space.
    pub(crate) fn search_picked(&mut self, at: usize, tx: &mut Tx) {
        let Some(list) = self.search_list() else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let edit = &mut search.edit;
        let (value_range, value) = match &list {
            FieldList::Filters(keywords) => {
                let Some(keyword) = keywords.get(at) else {
                    return;
                };
                edit.replace_text(keyword_text(*keyword));
                edit.set_caret(edit.text.len());
                search.highlight = 0;
                self.search_edited(tx);
                tx.focus(SEARCH_FIELD_KEY, true);
                return;
            }
            FieldList::Values {
                value_range, items, ..
            } => {
                let Some(item) = items.get(at) else {
                    return;
                };
                (value_range, written(item))
            }
            FieldList::Dates {
                value_range, items, ..
            } => {
                let Some(item) = items.get(at) else {
                    return;
                };
                (value_range, item.binding.as_str())
            }
        };
        let follows_space = edit
            .text
            .get(value_range.end..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_whitespace);
        let mut value = value.to_owned();
        if !follows_space {
            value.push(' ');
        }
        edit.set_selection(value_range.start, value_range.end);
        edit.insert(&value);
        search.highlight = 0;
        self.search_edited(tx);
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// Clear filters, when nothing was found: the words stay.
    pub(crate) fn search_clear_filters(&mut self, tx: &mut Tx) {
        let Some(draft) = self.search_draft() else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let words: Vec<&str> = draft
            .words
            .iter()
            .filter_map(|range| search.edit.text.get(range.clone()))
            .collect();
        let text = words.join(" ");
        search.edit.replace_text(text);
        search.edit.set_caret(search.edit.text.len());
        self.search_now();
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// Enter or Tab with the caret at the end of a filter's bound value
    /// (typed out in full, not picked): a space follows it, as after a
    /// pick. Returns whether it did.
    fn end_typed_value(&mut self, tx: &mut Tx) -> bool {
        let Some(draft) = self.search_draft() else {
            return false;
        };
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let caret = search.edit.range().1;
        let ends_bound = draft.filter_at(caret).is_some_and(|filter| {
            filter.value_range.end == caret
                && matches!(
                    filter.resolved,
                    FilterValue::Person(_)
                        | FilterValue::Channel(_)
                        | FilterValue::Has(_)
                        | FilterValue::Date(_)
                )
        });
        let spaced = search
            .edit
            .text
            .get(caret..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_whitespace);
        if !ends_bound || spaced {
            return false;
        }
        search.edit.set_caret(caret);
        search.edit.insert(" ");
        self.search_edited(tx);
        true
    }
}
