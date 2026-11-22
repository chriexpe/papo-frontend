//! Pairs and wraps in the field (`docs/design.md` §3, "Markdown in place"):
//! the SDK's pair closing (`on_pair_change`, `skip_pair_closer`,
//! `track_pair_change`) and selection formatting (`apply_selection_format`)
//! applied to Rune's `TextEdit`.
//!
//! Only a typed character or Backspace closes a pair; the field tells this
//! module which edits those were. Every result is applied as one ordinary
//! edit of the range that changed, never by replacing the whole text, so
//! undo keeps its history.

use std::ops::Range;

use papo_sdk_ui::message_content::{
    PairClosing, SelectionFormat, TextChangeSource, apply_selection_format, on_pair_change,
    skip_pair_closer, track_pair_change,
};
use rune::{Key, KeyEvent, TextEdit};

/// The pairs open in one field, and its text as of the last change seen.
#[derive(Debug, Default)]
pub(crate) struct Pairs {
    before: String,
    pairing: Option<PairClosing>,
}

impl Pairs {
    /// The field's text was replaced (a draft restored, a message sent): no
    /// pair is open.
    pub(crate) fn reset(&mut self, text: &str) {
        text.clone_into(&mut self.before);
        self.pairing = None;
    }

    /// The field's text changed; `typed` when a typed character or
    /// Backspace changed it. A typed opener gets its closer, a typed closer
    /// steps over the one waiting, and Backspace in an empty pair removes
    /// both. Any other change only moves the pairs along.
    pub(crate) fn changed(&mut self, edit: &mut TextEdit, typed: bool) {
        let (start, end) = edit.range();
        if typed && start == end {
            let result = on_pair_change(
                &self.before,
                &edit.text,
                end,
                TextChangeSource::Typing,
                &mut self.pairing,
            );
            if let Some(result) = result {
                apply(edit, &result.text, result.byte_caret..result.byte_caret);
            }
        } else {
            track_pair_change(&self.before, &edit.text, &mut self.pairing);
        }
        edit.text.clone_into(&mut self.before);
    }

    /// Tab: the caret steps past the innermost closer waiting. Whether
    /// there was one to step past.
    pub(crate) fn skip(&mut self, edit: &mut TextEdit) -> bool {
        let (start, end) = edit.range();
        if start != end {
            return false;
        }
        let Some(result) = skip_pair_closer(&edit.text, end, &mut self.pairing) else {
            return false;
        };
        apply(edit, &result.text, result.byte_caret..result.byte_caret);
        edit.text.clone_into(&mut self.before);
        true
    }
}

/// What a key does to a selection, when it wraps one (Christian,
/// 2026-11-22: one key, one style; bold is Ctrl+B only).
pub(crate) fn wrap_format(key: &KeyEvent) -> Option<SelectionFormat> {
    let mods = key.mods;
    match &key.key {
        Key::Text(text) if !mods.ctrl && !mods.cmd && !mods.alt => match text.as_str() {
            "*" => Some(SelectionFormat::Italic),
            "~" => Some(SelectionFormat::Strikethrough),
            "|" => Some(SelectionFormat::Spoiler),
            "`" => Some(SelectionFormat::Code),
            "[" => Some(SelectionFormat::Link { url: None }),
            _ => None,
        },
        Key::Command(_) if !mods.shift && !mods.alt => match key.edit_command() {
            Some('b') => Some(SelectionFormat::Bold),
            Some('i') => Some(SelectionFormat::Italic),
            _ => None,
        },
        _ => None,
    }
}

/// Wraps the field's selection in `format`, or unwraps it when it already
/// is. A bare caret wraps only with a shortcut (Ctrl+B leaves `**|**`);
/// a typed delimiter there is just typing. Whether the field changed.
pub(crate) fn wrap(edit: &mut TextEdit, format: &SelectionFormat, by_shortcut: bool) -> bool {
    let (start, end) = edit.range();
    if start == end && !by_shortcut {
        return false;
    }
    let Ok(result) = apply_selection_format(&edit.text, start..end, format) else {
        return false;
    };
    apply(edit, &result.text, result.byte_selection);
    true
}

/// Turns the field's text into `text` by replacing only the range that
/// differs, as one edit, then selects `selection`.
fn apply(edit: &mut TextEdit, text: &str, selection: Range<usize>) {
    let changed = differing(&edit.text, text);
    if let Some((old, new)) = changed {
        edit.set_selection(old.start, old.end);
        edit.insert(&text[new]);
    }
    edit.set_selection(selection.start, selection.end);
}

/// The byte ranges of `old` and `new` between their common start and
/// common end, on character boundaries; none when they are equal.
fn differing(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    if old == new {
        return None;
    }
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let room = old.len().min(new.len()) - prefix;
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .scan(0, |total, len| {
            *total += len;
            (*total <= room).then_some(len)
        })
        .sum();
    Some((prefix..old.len() - suffix, prefix..new.len() - suffix))
}

#[cfg(test)]
mod tests;
