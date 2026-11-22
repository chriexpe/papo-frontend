//! Mentions in the field: each one a Rune pill carrying the person's id, so
//! the field keeps it whole and in place across edits, undo and redo. On
//! send and save the pills become the SDK's bindings and the text its wire
//! form (`encode_composer_mentions`); a restored draft's wire form becomes
//! pills again (`decode_composer_mentions`, run where the names are).
//!
//! Also the @ query at the field's caret, found by the SDK's
//! `mention_query` among the field's pills, and picking one of the SDK's
//! ranked `mention_suggestions` (`docs/design.md` §3, "Composer surfaces").

use std::ops::Range;

use papo_sdk::{MentionTarget, Uuid};
use papo_sdk_ui::message_content::{
    ComposerMentionBinding, DecodedComposerMentions, encode_composer_mentions, utf8_range_to_utf16,
};
use papo_sdk_ui::{MentionQuery, MentionSuggestion, mention_query};
use rune::TextEdit;

/// The text that mentions everyone.
const EVERYONE: &str = "@everyone";

/// The SDK's bindings for the field's pills, in text order.
pub(crate) fn bindings(edit: &TextEdit) -> Vec<ComposerMentionBinding> {
    edit.pills()
        .filter_map(|(range, id)| {
            let id = Uuid::parse_str(id).ok()?;
            let utf16_range = utf8_range_to_utf16(&edit.text, &range)?;
            let text = edit.text.get(range.clone())?.to_owned();
            Some(ComposerMentionBinding {
                id,
                byte_range: range,
                utf16_range,
                text,
            })
        })
        .collect()
}

/// The field's text as sent and stored: each pill its mention token.
pub(crate) fn wire(edit: &TextEdit) -> String {
    let bindings = bindings(edit);
    if bindings.is_empty() {
        return edit.text.clone();
    }
    encode_composer_mentions(&edit.text, &bindings)
}

/// Puts `decoded` in the field with its mentions as pills, the caret at
/// the end and no undo history: a restored draft is where editing starts.
pub(crate) fn restore(edit: &mut TextEdit, decoded: DecodedComposerMentions) {
    edit.replace_text(decoded.text);
    pill(edit, decoded.bindings);
    edit.clear_history();
    edit.set_caret(edit.text.len());
}

/// Puts `decoded` (a message the SDK did not queue) before what the field
/// holds, on a line of its own, with its mentions as pills; the field's
/// own pills move along.
pub(crate) fn prepend(edit: &mut TextEdit, decoded: DecodedComposerMentions) {
    if edit.text.is_empty() {
        restore(edit, decoded);
        return;
    }
    edit.undo_group(|edit| {
        edit.set_caret(0);
        edit.insert(&format!("{}\n", decoded.text));
        pill(edit, decoded.bindings);
    });
    edit.set_caret(edit.text.len());
}

/// Makes each binding's range of the field's text a pill. The label is the
/// text already there, so no offset moves, unless Rune has to clean it (a
/// control character in a nickname) and it comes out shorter: back to
/// front, only ranges already done would move.
fn pill(edit: &mut TextEdit, bindings: Vec<ComposerMentionBinding>) {
    for binding in bindings.into_iter().rev() {
        edit.replace_with_pill(binding.byte_range, binding.id.to_string(), &binding.text);
    }
}

/// The @ query the field's caret ends, if any: a collapsed caret, outside
/// every mention already made.
pub(crate) fn query(edit: &TextEdit) -> Option<MentionQuery> {
    let (start, caret) = edit.range();
    if start != caret {
        return None;
    }
    let pills: Vec<Range<usize>> = edit.pills().map(|(range, _)| range).collect();
    mention_query(&edit.text, caret, &pills)
}

/// Replaces the query with the mention of `candidate` and a space after it.
pub(crate) fn pick(edit: &mut TextEdit, query: &MentionQuery, candidate: &MentionSuggestion) {
    match candidate.target {
        // The mention and its space are one undo step.
        MentionTarget::User(id) => edit.undo_group(|edit| {
            edit.replace_with_pill(
                query.byte_range.clone(),
                id.to_string(),
                &format!("@{}", candidate.name),
            );
            edit.insert(" ");
        }),
        MentionTarget::Everyone => {
            edit.set_selection(query.byte_range.start, query.byte_range.end);
            edit.insert(&format!("{EVERYONE} "));
        }
    }
}

#[cfg(test)]
mod tests;
