//! Pure composer mention encode/decode helpers delegating syntax to [`papo_sdk::mentions`].

use std::ops::Range;

use papo_sdk::{MentionBinding, Uuid};

use crate::message_content::preview::build_utf16_offset_map;

/// One editable mention span inside a draft carrying both UTF-8 byte and UTF-16 ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerMentionBinding {
    /// Canonical backend user UUID.
    pub id: Uuid,
    /// Exact UTF-8 byte span in the composer display text.
    pub byte_range: Range<usize>,
    /// Corresponding UTF-16 code-unit span for JVM/Android/macOS native text views.
    pub utf16_range: Range<usize>,
    /// Expected exact display slice (`&text[byte_range]`, typically `@display_name`).
    pub text: String,
}

/// Editable composer text produced when decoding canonical message wire mentions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedComposerMentions {
    /// Draft display text with known IDs substituted and unknown IDs kept verbatim.
    pub text: String,
    /// Active bindings for every resolved mention in `text`.
    pub bindings: Vec<ComposerMentionBinding>,
}

/// Converts a UTF-8 byte offset in `text` into a UTF-16 code-unit offset.
#[must_use]
pub fn utf8_offset_to_utf16(text: &str, byte_offset: usize) -> Option<usize> {
    if !text.is_char_boundary(byte_offset) {
        return None;
    }
    Some(text[..byte_offset].encode_utf16().count())
}

/// Converts a UTF-8 byte range in `text` into a UTF-16 code-unit range.
#[must_use]
pub fn utf8_range_to_utf16(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return None;
    }
    let start = text[..range.start].encode_utf16().count();
    let len = text[range.start..range.end].encode_utf16().count();
    Some(start..start + len)
}

/// Converts a UTF-16 code-unit offset in `text` back to a UTF-8 byte offset on a scalar boundary.
///
/// Returns `None` if `utf16_offset` splits a surrogate pair or exceeds the UTF-16 length of `text`.
#[must_use]
pub fn utf16_offset_to_utf8(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut current_utf16 = 0usize;
    if utf16_offset == 0 {
        return Some(0);
    }
    for (byte, ch) in text.char_indices() {
        if current_utf16 == utf16_offset {
            return Some(byte);
        }
        current_utf16 += ch.len_utf16();
        if current_utf16 > utf16_offset {
            return None;
        }
    }
    (current_utf16 == utf16_offset).then_some(text.len())
}

/// Converts a UTF-16 code-unit range in `text` back to a UTF-8 byte range.
#[must_use]
pub fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end {
        return None;
    }
    let start = utf16_offset_to_utf8(text, range.start)?;
    let end = utf16_offset_to_utf8(text, range.end)?;
    Some(start..end)
}

/// Decodes canonical wire tokens (`@mention(<@uuid>)`) into editable composer text and dual-offset
/// bindings, preserving unknown user IDs verbatim.
pub fn decode_composer_mentions<'a>(
    wire: &str,
    resolve: impl FnMut(Uuid) -> Option<&'a str>,
) -> DecodedComposerMentions {
    let decoded = papo_sdk::decode_mentions_for_editing(wire, resolve);
    if decoded.bindings.is_empty() {
        return DecodedComposerMentions {
            text: decoded.text,
            bindings: Vec::new(),
        };
    }

    let utf16_map = build_utf16_offset_map(&decoded.text);
    let bindings = decoded
        .bindings
        .into_iter()
        .map(|binding| ComposerMentionBinding {
            id: binding.id,
            utf16_range: utf16_map[binding.start]..utf16_map[binding.end],
            byte_range: binding.start..binding.end,
            text: binding.text,
        })
        .collect();

    DecodedComposerMentions {
        text: decoded.text,
        bindings,
    }
}

/// Encodes validated composer mention bindings back into canonical `@mention(<@uuid>)` tokens.
#[must_use]
pub fn encode_composer_mentions(display: &str, bindings: &[ComposerMentionBinding]) -> String {
    let core_bindings: Vec<MentionBinding> = bindings
        .iter()
        .map(|binding| MentionBinding {
            id: binding.id,
            start: binding.byte_range.start,
            end: binding.byte_range.end,
            text: binding.text.clone(),
        })
        .collect();
    papo_sdk::encode_mentions_with_bindings(display, &core_bindings)
}
