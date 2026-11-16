//! Pure selection wrap/unwrap format assist for composer selections.

use std::ops::Range;

use crate::message_content::composer_mentions::utf8_range_to_utf16;

/// Formatting action applied to the active composer selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionFormat {
    /// Bold wrapper (`**...**`).
    Bold,
    /// Italic wrapper (`*...*`).
    Italic,
    /// Strikethrough wrapper (`~~...~~`).
    Strikethrough,
    /// Inline code (`` `...` ``) or multiline fenced code (```` ```\n...\n``` ````).
    Code,
    /// Spoiler wrapper (`||...||`).
    Spoiler,
    /// Markdown link (`[text](url)`), placing caret in `()` when `url` is `None`.
    Link {
        /// Optional destination URL inserted inside `(...)`.
        url: Option<String>,
    },
}

/// Invalid UTF-8 boundary or out-of-range selection supplied to [`apply_selection_format`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidSelection;

impl std::fmt::Display for InvalidSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("selection range is out of bounds or not on UTF-8 boundaries")
    }
}

impl std::error::Error for InvalidSelection {}

/// Updated draft text plus replacement UTF-8 byte and UTF-16 selection ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionEdit {
    /// Resulting composer buffer after wrapping or unwrapping.
    pub text: String,
    /// Updated selection in UTF-8 byte coordinates.
    pub byte_selection: Range<usize>,
    /// Updated selection in UTF-16 code-unit coordinates.
    pub utf16_selection: Range<usize>,
}

/// Wraps `selection` with `format` markers, or unwraps if already formatted inside or around it.
///
/// # Errors
/// Returns [`InvalidSelection`] if `selection.start > selection.end`, exceeds `text.len()`,
/// or falls inside a multi-byte UTF-8 character.
pub fn apply_selection_format(
    text: &str,
    selection: Range<usize>,
    format: &SelectionFormat,
) -> Result<SelectionEdit, InvalidSelection> {
    if selection.start > selection.end
        || selection.end > text.len()
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return Err(InvalidSelection);
    }

    let edit = match format {
        SelectionFormat::Bold => toggle_symmetric(text, selection, "**"),
        SelectionFormat::Italic => toggle_italic(text, selection),
        SelectionFormat::Strikethrough => toggle_symmetric(text, selection, "~~"),
        SelectionFormat::Spoiler => toggle_symmetric(text, selection, "||"),
        SelectionFormat::Code => toggle_code(text, selection),
        SelectionFormat::Link { url } => toggle_link(text, selection, url.as_deref()),
    };

    Ok(edit)
}

fn make_edit(text: String, byte_selection: Range<usize>) -> SelectionEdit {
    let utf16_selection = utf8_range_to_utf16(&text, &byte_selection).unwrap_or(0..0);
    SelectionEdit {
        text,
        byte_selection,
        utf16_selection,
    }
}

fn toggle_symmetric(text: &str, selection: Range<usize>, marker: &str) -> SelectionEdit {
    let len = marker.len();
    let selected = &text[selection.clone()];
    let before = &text[..selection.start];
    let after = &text[selection.end..];

    if selected.len() >= len * 2 && selected.starts_with(marker) && selected.ends_with(marker) {
        let inner = &selected[len..selected.len() - len];
        let mut out = String::with_capacity(text.len() - len * 2);
        out.push_str(before);
        out.push_str(inner);
        out.push_str(after);
        return make_edit(out, selection.start..selection.start + inner.len());
    }

    if before.ends_with(marker) && after.starts_with(marker) {
        let mut out = String::with_capacity(text.len() - len * 2);
        out.push_str(&before[..before.len() - len]);
        out.push_str(selected);
        out.push_str(&after[len..]);
        let start = selection.start - len;
        return make_edit(out, start..start + selected.len());
    }

    let mut out = String::with_capacity(text.len() + len * 2);
    out.push_str(before);
    out.push_str(marker);
    out.push_str(selected);
    out.push_str(marker);
    out.push_str(after);
    let inner_start = selection.start + len;
    make_edit(out, inner_start..inner_start + selected.len())
}

fn has_single_star(prefix: &str, suffix: &str) -> (bool, bool) {
    let left = prefix.ends_with('*') && (!prefix.ends_with("**") || prefix.ends_with("***"));
    let right = suffix.starts_with('*') && (!suffix.starts_with("**") || suffix.starts_with("***"));
    (left, right)
}

fn toggle_italic(text: &str, selection: Range<usize>) -> SelectionEdit {
    let selected = &text[selection.clone()];
    let before = &text[..selection.start];
    let after = &text[selection.end..];

    if selected.len() >= 2 {
        let starts_single = selected.starts_with('*')
            && (!selected.starts_with("**") || selected.starts_with("***"));
        let ends_single =
            selected.ends_with('*') && (!selected.ends_with("**") || selected.ends_with("***"));
        if starts_single && ends_single {
            let inner = &selected[1..selected.len() - 1];
            let mut out = String::with_capacity(text.len() - 2);
            out.push_str(before);
            out.push_str(inner);
            out.push_str(after);
            return make_edit(out, selection.start..selection.start + inner.len());
        }
    }

    let (left, right) = has_single_star(before, after);
    if left && right {
        let mut out = String::with_capacity(text.len() - 2);
        out.push_str(&before[..before.len() - 1]);
        out.push_str(selected);
        out.push_str(&after[1..]);
        let start = selection.start - 1;
        return make_edit(out, start..start + selected.len());
    }

    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(before);
    out.push('*');
    out.push_str(selected);
    out.push('*');
    out.push_str(after);
    let inner_start = selection.start + 1;
    make_edit(out, inner_start..inner_start + selected.len())
}

fn toggle_code(text: &str, selection: Range<usize>) -> SelectionEdit {
    let selected = &text[selection.clone()];
    let before = &text[..selection.start];
    let after = &text[selection.end..];

    // Check multiline fenced wrap/unwrap first.
    if selected.starts_with("```\n") && selected.ends_with("\n```") && selected.len() >= 8 {
        let inner = &selected[4..selected.len() - 4];
        let mut out = String::with_capacity(text.len() - 8);
        out.push_str(before);
        out.push_str(inner);
        out.push_str(after);
        return make_edit(out, selection.start..selection.start + inner.len());
    }
    if before.ends_with("```\n") && after.starts_with("\n```") {
        let mut out = String::with_capacity(text.len() - 8);
        out.push_str(&before[..before.len() - 4]);
        out.push_str(selected);
        out.push_str(&after[4..]);
        let start = selection.start - 4;
        return make_edit(out, start..start + selected.len());
    }

    // Multiline selections wrap in fenced code blocks; single-line uses inline backticks.
    if selected.contains('\n') {
        let mut out = String::with_capacity(text.len() + 8);
        out.push_str(before);
        out.push_str("```\n");
        out.push_str(selected);
        out.push_str("\n```");
        out.push_str(after);
        let inner_start = selection.start + 4;
        return make_edit(out, inner_start..inner_start + selected.len());
    }

    toggle_symmetric(text, selection, "`")
}

fn toggle_link(text: &str, selection: Range<usize>, url: Option<&str>) -> SelectionEdit {
    let selected = &text[selection.clone()];
    let before = &text[..selection.start];
    let after = &text[selection.end..];

    // Case 1: The selection itself is a complete `[label](dest)` markdown link -> unwrap to `label`.
    if let Some(label) = unwrap_markdown_link(selected) {
        let mut out = String::with_capacity(text.len() - (selected.len() - label.len()));
        out.push_str(before);
        out.push_str(label);
        out.push_str(after);
        return make_edit(out, selection.start..selection.start + label.len());
    }

    // Case 2: Selection covers the label inside `[` and `](dest)` -> unwrap to label.
    if before.ends_with('[')
        && after.starts_with("](")
        && let Some(close_rel) = find_closing_link_paren(&after[2..])
    {
        let mut out = String::with_capacity(text.len() - 3 - close_rel);
        out.push_str(&before[..before.len() - 1]);
        out.push_str(selected);
        out.push_str(&after[3 + close_rel..]);
        let start = selection.start - 1;
        return make_edit(out, start..start + selected.len());
    }

    let mut out = String::with_capacity(text.len() + 4 + url.map_or(0, str::len));
    out.push_str(before);
    out.push('[');
    out.push_str(selected);
    out.push_str("](");
    let url_start = out.len();
    if let Some(href) = url {
        out.push_str(href);
    }
    let url_end = out.len();
    out.push(')');
    out.push_str(after);

    let next_selection = if selected.is_empty() {
        let caret = selection.start + 1;
        caret..caret
    } else if url.is_some() {
        selection.start..url_end + 1
    } else {
        url_start..url_start
    };
    make_edit(out, next_selection)
}

fn unwrap_markdown_link(slice: &str) -> Option<&str> {
    let rest = slice.strip_prefix('[')?;
    let sep = rest.find("](")?;
    let label = &rest[..sep];
    if label.contains(['[', ']']) {
        return None;
    }
    let after_open = &rest[sep + 2..];
    let close_idx = find_closing_link_paren(after_open)?;
    (close_idx + 1 == after_open.len()).then_some(label)
}

fn find_closing_link_paren(after_open_paren: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (idx, byte) in after_open_paren.bytes().enumerate() {
        match byte {
            b'\n' | b'\r' => return None,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}
