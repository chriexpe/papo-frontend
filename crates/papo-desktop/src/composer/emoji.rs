//! Custom emoji in the field (`docs/design.md` §16.2, §9 Inline): each one a Rune pill whose
//! label is its `:name:` and whose id names the emoji, drawn as its image at line height
//! (`El::pill_inline_image`). The label is the text sent and saved, so the wire form needs no
//! encoding, and a restored draft's `:name:` becomes a pill again here. A `:name:` typed whole
//! becomes one as its closing colon is typed; code never does, as the message would not draw
//! one there.

use std::ops::Range;

use papo_sdk::Uuid;
use papo_sdk_ui::message_content::{CustomEmojis, PreviewSpan, PreviewSpanKind};
use rune::TextEdit;

/// What an emoji pill's id starts with, apart from a mention's (a bare UUID).
const PILL_PREFIX: &str = "emoji:";

/// The pill id of emoji `id`.
pub(crate) fn pill_id(id: Uuid) -> String {
    format!("{PILL_PREFIX}{id}")
}

/// The emoji a pill names, when it is an emoji's.
pub(crate) fn of_pill(pill: &str) -> Option<Uuid> {
    pill.strip_prefix(PILL_PREFIX)
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// The emoji the field holds, in order.
pub(crate) fn ids(edit: &TextEdit) -> Vec<Uuid> {
    edit.pills().filter_map(|(_, id)| of_pill(id)).collect()
}

/// A `:name:` the caret just closed by typing its colon becomes the emoji's pill. Returns
/// whether it did.
pub(crate) fn bind_typed(edit: &mut TextEdit, emojis: &CustomEmojis, code: &[PreviewSpan]) -> bool {
    let (start, caret) = edit.range();
    if start != caret || !edit.text[..caret].ends_with(':') {
        return false;
    }
    let Some(found) = name_ending_at(&edit.text, caret, emojis) else {
        return false;
    };
    if in_code(&found.range, code) || overlaps_pill(edit, &found.range) {
        return false;
    }
    edit.replace_with_pill(found.range, pill_id(found.id), &found.label);
    true
}

/// The picker's pick (§16.2): custom emoji `id`, named `name`, goes in at the caret, over any
/// selection, as its pill; inside code, as its `:name:` in words, as typing it there leaves it.
/// One undo step.
pub(crate) fn insert(edit: &mut TextEdit, id: Uuid, name: &str, code: &[PreviewSpan]) {
    let (start, end) = edit.range();
    let at = start.min(end);
    let label = format!(":{name}:");
    let in_code = in_code(&(at..at), code);
    edit.undo_group(|edit| {
        edit.insert(&label);
        if !in_code {
            edit.replace_with_pill(at..at + label.len(), pill_id(id), &label);
        }
    });
}

/// Every `:name:` of the field's text outside code and pills becomes the emoji's pill: a
/// restored draft, or a message taken back for editing. Offsets don't move: each label is the
/// text already there.
pub(crate) fn bind_all(edit: &mut TextEdit, emojis: &CustomEmojis, code: &[PreviewSpan]) {
    if emojis.refs().is_empty() {
        return;
    }
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(open) = edit.text[at..].find(':').map(|offset| at + offset) {
        match name_starting_at(&edit.text, open, emojis) {
            Some(name) => {
                at = name.range.end;
                found.push(name);
            }
            None => at = open + 1,
        }
    }
    // One undo step, and the caret and selection stay where they were: offsets don't move,
    // as each label is the text already there.
    let (anchor, caret) = edit.range();
    edit.undo_group(|edit| {
        // Back to front: only ranges already done would move, and none do.
        for name in found.into_iter().rev() {
            if !in_code(&name.range, code) && !overlaps_pill(edit, &name.range) {
                edit.replace_with_pill(name.range, pill_id(name.id), &name.label);
            }
        }
    });
    edit.set_selection(anchor, caret);
}

/// A `:name:` in the text that names an emoji.
struct Found {
    range: Range<usize>,
    id: Uuid,
    label: String,
}

/// The `:name:` that ends at byte `end` (just after its closing colon), if one names an emoji.
fn name_ending_at(text: &str, end: usize, emojis: &CustomEmojis) -> Option<Found> {
    let inner_end = end.checked_sub(1)?;
    let open = text[..inner_end].rfind(':')?;
    let found = name_starting_at(text, open, emojis)?;
    (found.range.end == end).then_some(found)
}

/// The `:name:` whose opening colon is at byte `open`, if one names an emoji.
fn name_starting_at(text: &str, open: usize, emojis: &CustomEmojis) -> Option<Found> {
    let after = &text[open + 1..];
    let close = after.find(':')?;
    let name = &after[..close];
    if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let entry = emojis.refs().iter().find(|entry| entry.name == name)?;
    let end = open + 1 + close + 1;
    Some(Found {
        range: open..end,
        id: entry.id,
        label: text[open..end].to_owned(),
    })
}

fn in_code(range: &Range<usize>, code: &[PreviewSpan]) -> bool {
    code.iter().any(|span| {
        matches!(
            span.kind,
            PreviewSpanKind::Code | PreviewSpanKind::CodeBlock
        ) && span.byte_range.start < range.end
            && range.start < span.byte_range.end
    })
}

fn overlaps_pill(edit: &TextEdit, range: &Range<usize>) -> bool {
    edit.pills()
        .any(|(pill, _)| pill.start < range.end && range.start < pill.end)
}

#[cfg(test)]
mod tests {
    use papo_sdk::{CustomEmoji, EmojiCatalog};
    use papo_sdk_ui::message_content::preview_spans;

    use super::*;

    fn emojis() -> CustomEmojis {
        CustomEmojis::new(&EmojiCatalog {
            revision: 5,
            emojis: vec![CustomEmoji {
                id: Uuid::from_u128(9),
                name: "gato".into(),
                format: "png".into(),
                created_at: "t".into(),
            }],
        })
    }

    fn typed(text: &str) -> TextEdit {
        let mut edit = TextEdit::new("").rich();
        edit.insert(text);
        edit
    }

    #[test]
    fn a_typed_name_becomes_its_emoji_as_its_colon_closes_it() {
        let mut edit = typed("oi :gato:");
        let code = preview_spans(&edit.text);
        assert!(bind_typed(&mut edit, &emojis(), &code));
        assert_eq!(ids(&edit), [Uuid::from_u128(9)]);
        assert_eq!(edit.text, "oi :gato:", "the label is the text sent");
        let mut unknown = typed("oi :cao:");
        let code = preview_spans(&unknown.text);
        assert!(!bind_typed(&mut unknown, &emojis(), &code));
    }

    #[test]
    fn a_restored_draft_binds_every_name_outside_code() {
        let mut edit = typed(":gato: e `:gato:` e :gato:");
        let code = preview_spans(&edit.text);
        bind_all(&mut edit, &emojis(), &code);
        assert_eq!(ids(&edit).len(), 2);
        let pills: Vec<_> = edit.pills().map(|(range, _)| range).collect();
        assert_eq!(pills, [0..6, 20..26]);
    }

    #[test]
    fn binding_keeps_the_caret_and_is_one_undo_step() {
        let mut edit = typed(":gato: e :gato: tudo");
        edit.set_caret(9);
        let code = preview_spans(&edit.text);
        bind_all(&mut edit, &emojis(), &code);
        assert_eq!(edit.range(), (9, 9), "the caret stays");
        assert_eq!(ids(&edit).len(), 2);
        edit.undo();
        assert!(ids(&edit).is_empty(), "one undo takes both back");
    }
}
