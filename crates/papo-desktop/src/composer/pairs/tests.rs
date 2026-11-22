//! Pairs closing, stepping over and wrapping on a real `TextEdit`, with
//! typing simulated as the field edits it.

use rune::{Key, KeyEvent, Mods, TextEdit};

use super::{Pairs, differing, wrap, wrap_format};

/// The field's text with `|` at the caret.
fn shown(edit: &TextEdit) -> String {
    let (start, end) = edit.range();
    assert_eq!(start, end, "a caret, not a selection");
    format!("{}|{}", &edit.text[..end], &edit.text[end..])
}

/// Types each character of `text` as the keyboard would.
fn type_in(edit: &mut TextEdit, pairs: &mut Pairs, text: &str) {
    for ch in text.chars() {
        edit.insert(ch.encode_utf8(&mut [0; 4]));
        pairs.changed(edit, true);
    }
}

fn backspace(edit: &mut TextEdit, pairs: &mut Pairs) {
    let (_, end) = edit.range();
    let start = edit.text[..end]
        .char_indices()
        .next_back()
        .map_or(0, |(at, _)| at);
    edit.set_selection(start, end);
    edit.insert("");
    pairs.changed(edit, true);
}

fn field() -> (TextEdit, Pairs) {
    (TextEdit::new(""), Pairs::default())
}

#[test]
fn a_typed_opener_gets_its_closer() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "**");
    assert_eq!(shown(&edit), "**|**");
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "(");
    assert_eq!(shown(&edit), "(|)");
}

#[test]
fn a_single_star_never_closes() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "* item");
    assert_eq!(shown(&edit), "* item|");
}

#[test]
fn typing_the_closer_steps_over_it_and_leaves_a_space() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "**oi**");
    assert_eq!(shown(&edit), "**oi** |");
}

#[test]
fn a_bracket_steps_over_without_a_space() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "(a)");
    assert_eq!(shown(&edit), "(a)|");
}

#[test]
fn backspace_in_an_empty_pair_removes_both() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "x ||");
    assert_eq!(shown(&edit), "x |||||");
    backspace(&mut edit, &mut pairs);
    // One of the opener's characters goes with the closer; a single `|`
    // opens nothing.
    assert_eq!(edit.text, "x |");
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "(");
    backspace(&mut edit, &mut pairs);
    assert_eq!(shown(&edit), "|");
}

#[test]
fn a_paste_never_closes_a_pair() {
    let (mut edit, mut pairs) = field();
    edit.insert("**");
    pairs.changed(&mut edit, false);
    assert_eq!(shown(&edit), "**|");
}

#[test]
fn tab_steps_past_the_closer() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "**oi");
    assert!(pairs.skip(&mut edit));
    assert_eq!(shown(&edit), "**oi** |");
    assert!(!pairs.skip(&mut edit), "nothing left to step past");
}

#[test]
fn undo_takes_back_the_closer_without_losing_the_rest() {
    let (mut edit, mut pairs) = field();
    type_in(&mut edit, &mut pairs, "a **");
    assert_eq!(edit.text, "a ****");
    edit.undo();
    assert!(edit.text.starts_with("a "), "{:?}", edit.text);
    assert!(edit.has_history(), "the history survived the closer");
}

#[test]
fn a_restored_draft_opens_no_pair() {
    let (mut edit, mut pairs) = field();
    edit.replace_text("**oi");
    pairs.reset(&edit.text);
    type_in(&mut edit, &mut pairs, "*");
    assert_eq!(shown(&edit), "**oi*|");
}

fn key(text: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(text.into()),
        mods: Mods::default(),
    }
}

/// `text` with "oi" selected.
fn selecting_oi(text: &str) -> TextEdit {
    let mut edit = TextEdit::new(text);
    let at = text.find("oi").unwrap();
    edit.set_selection(at, at + 2);
    edit
}

#[test]
fn a_delimiter_typed_over_a_selection_wraps_it_and_again_unwraps_it() {
    let mut edit = selecting_oi("a oi b");
    let italic = wrap_format(&key("*")).unwrap();
    assert!(wrap(&mut edit, &italic, false));
    assert_eq!(edit.text, "a *oi* b");
    assert_eq!(edit.selected(), "oi");
    assert!(wrap(&mut edit, &italic, false));
    assert_eq!(edit.text, "a oi b");
}

#[test]
fn each_key_wraps_in_its_own_style() {
    for (typed, wrapped) in [("~", "~~oi~~"), ("|", "||oi||"), ("`", "`oi`")] {
        let mut edit = selecting_oi("oi");
        assert!(wrap(&mut edit, &wrap_format(&key(typed)).unwrap(), false));
        assert_eq!(edit.text, wrapped);
    }
    let mut edit = selecting_oi("oi");
    assert!(wrap(&mut edit, &wrap_format(&key("[")).unwrap(), false));
    assert_eq!(shown(&edit), "[oi](|)");
}

#[test]
fn a_delimiter_typed_at_a_caret_is_just_typing() {
    let mut edit = TextEdit::new("oi");
    let italic = wrap_format(&key("*")).unwrap();
    assert!(!wrap(&mut edit, &italic, false));
    assert_eq!(edit.text, "oi");
}

#[test]
fn only_wrapping_keys_wrap() {
    assert!(wrap_format(&key("a")).is_none());
    assert!(wrap_format(&key("_")).is_none(), "snake_case stays typing");
}

#[test]
fn only_the_middle_differs() {
    assert_eq!(differing("ab", "ab"), None);
    assert_eq!(differing("a**", "a****"), Some((3..3, 3..5)));
    assert_eq!(differing("ação", "açã!o"), Some((5..5, 5..6)));
    assert_eq!(differing("aa", "a"), Some((1..2, 1..1)));
}
