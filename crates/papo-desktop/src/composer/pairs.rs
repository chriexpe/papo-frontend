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

/// A key that types: only a change it makes may close a pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Keystroke {
    /// A character, or what an `AltGr` or Option chord types.
    Text(String),
    Backspace,
}

impl Keystroke {
    /// Whether `before` becoming `after` is what this key does: its text
    /// inserted, or one character removed.
    fn made(&self, before: &str, after: &str) -> bool {
        let Some((old, new)) = differing(before, after) else {
            return false;
        };
        match self {
            Keystroke::Text(text) => old.is_empty() && after[new] == **text,
            Keystroke::Backspace => new.is_empty() && before[old].chars().count() == 1,
        }
    }
}

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

    /// The field's text changed; `key` the typing key that reached it in
    /// the same event, if any. When the change is that key's own, a typed
    /// opener gets its closer, a typed closer steps over the one waiting,
    /// and Backspace in an empty pair removes both. Any other change (a
    /// paste, an input method's commit, undo) only moves the pairs along.
    pub(crate) fn changed(&mut self, edit: &mut TextEdit, key: Option<&Keystroke>) {
        let (start, end) = edit.range();
        let typed = key.is_some_and(|key| key.made(&self.before, &edit.text));
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
        // AltGr (Ctrl+Alt off macOS) and Option still type: only a command
        // chord is not typing.
        Key::Text(text) if !key.is_command_chord() => match text.as_str() {
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
/// differs, as one undo step, then selects `selection`.
///
/// Rune drops the pills (mentions) an edit replaces. A pair or a wrap only
/// adds or removes markers around text, so each pill's label comes through
/// unchanged: it is found again in order and made a pill again.
fn apply(edit: &mut TextEdit, text: &str, selection: Range<usize>) {
    let Some((mut old, mut new)) = differing(&edit.text, text) else {
        edit.set_selection(selection.start, selection.end);
        return;
    };
    let pills: Vec<(Range<usize>, String)> = edit
        .pills()
        .filter(|(range, _)| range.start < old.end && range.end > old.start)
        .map(|(range, id)| (range, id.to_owned()))
        .collect();
    // A pill cut by the range joins it whole; the grown part is common to
    // both texts, so both ranges grow alike.
    if let (Some(first), Some(last)) = (pills.first(), pills.last()) {
        let grow_start = old.start.saturating_sub(first.0.start);
        let grow_end = last.0.end.saturating_sub(old.end);
        old = old.start - grow_start..old.end + grow_end;
        new = new.start - grow_start..new.end + grow_end;
    }
    let labels: Vec<(String, String)> = pills
        .into_iter()
        .map(|(range, id)| (id, edit.text[range].to_owned()))
        .collect();
    edit.undo_group(|edit| {
        edit.set_selection(old.start, old.end);
        edit.insert(&text[new.clone()]);
        let end = old.start + new.len();
        let mut from = old.start;
        for (id, label) in labels {
            let Some(at) = edit.text.get(from..end).and_then(|rest| rest.find(&label)) else {
                continue;
            };
            let start = from + at;
            edit.replace_with_pill(start..start + label.len(), id, &label);
            from = start + label.len();
        }
    });
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
