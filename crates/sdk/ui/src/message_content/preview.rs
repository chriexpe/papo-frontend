//! Source-preserving composer Markdown preview spans with UTF-8 byte and UTF-16 ranges.

use std::{borrow::Cow, ops::Range};

use papo_sdk::{MentionTarget, Uuid};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::message_content::{
    composer_mentions::ComposerMentionBinding,
    model::{CustomEmojiRef, MAX_NESTING_DEPTH},
    preprocess::{heading_level, is_safe_href, markdown_options, odd_trailing_backslashes},
    tokens::match_custom_emoji_at,
};

const MAX_PREVIEW_BYTES: usize = 128 * 1024;
const MAX_PREVIEW_SPANS: usize = 2048;

/// Kind of styling span highlighted in the message composer without rewriting source offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewSpanKind {
    /// Heading (`1..=6`).
    Heading(u8),
    /// Bold emphasis (`**...**`, `__...__`).
    Bold,
    /// Italic emphasis (`*...*`, `_..._`).
    Italic,
    /// Strikethrough (`~...~`, `~~...~~`).
    Strikethrough,
    /// Inline code span (`` `...` ``).
    Code,
    /// Fenced or indented code block.
    CodeBlock,
    /// Inline spoiler (`||...||`).
    Spoiler,
    /// Link with an allowed `http`, `https` or `mailto` destination.
    Link,
    /// Block quote (`> ...`).
    BlockQuote,
    /// Canonical or bound mention token highlighted in the composer.
    Mention(MentionTarget),
    /// Recognized server custom emoji `:shortcode:` matched outside code/links.
    CustomEmoji(Uuid),
}

/// One composer preview span with both UTF-8 byte offsets and UTF-16 code-unit offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewSpan {
    /// Highlight category for this source span.
    pub kind: PreviewSpanKind,
    /// Half-open UTF-8 byte range in the original composer string.
    pub byte_range: Range<usize>,
    /// Corresponding half-open UTF-16 code-unit range for JVM/Android/macOS editors.
    pub utf16_range: Range<usize>,
}

/// Computes source-preserving live preview spans with both UTF-8 byte and UTF-16 ranges.
#[must_use]
pub fn preview_spans(source: &str) -> Vec<PreviewSpan> {
    preview_spans_with_bindings(source, &[])
}

/// Computes live preview spans while masking bound editable display mentions (`@Name`) so names
/// containing `*` or `_` do not perturb `CommonMark` emphasis across the rest of the draft.
#[must_use]
pub fn preview_spans_with_bindings(
    source: &str,
    bindings: &[ComposerMentionBinding],
) -> Vec<PreviewSpan> {
    preview_spans_with_bindings_and_emojis(source, bindings, &[])
}

/// Computes live preview spans while masking bound display mentions and recognizing `:shortcode:`
/// spans from `custom_emojis` outside code blocks, inline code, and links.
#[must_use]
pub fn preview_spans_with_bindings_and_emojis(
    source: &str,
    bindings: &[ComposerMentionBinding],
    custom_emojis: &[CustomEmojiRef],
) -> Vec<PreviewSpan> {
    if source.len() > MAX_PREVIEW_BYTES || source.is_empty() {
        return Vec::new();
    }

    let masked = mask_bindings(source, bindings);
    let parse_source = masked.as_ref();

    let Some(mut raw) = collect_markdown_spans(parse_source) else {
        return Vec::new();
    };
    if !append_spoiler_and_mention_spans(source, parse_source, bindings, custom_emojis, &mut raw) {
        return Vec::new();
    }

    let utf16_map = build_utf16_offset_map(source);
    raw.into_iter()
        .map(|(kind, byte_range)| PreviewSpan {
            kind,
            utf16_range: utf16_map[byte_range.start]..utf16_map[byte_range.end],
            byte_range,
        })
        .collect()
}

fn collect_markdown_spans(source: &str) -> Option<Vec<(PreviewSpanKind, Range<usize>)>> {
    let mut result = Vec::new();
    let mut depth = 0usize;

    for (event, mut range) in Parser::new_ext(source, markdown_options()).into_offset_iter() {
        let kind = match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return None;
                }
                match tag {
                    Tag::Heading { level, .. } => {
                        while range.end > range.start
                            && matches!(source.as_bytes()[range.end - 1], b'\n' | b'\r')
                        {
                            range.end -= 1;
                        }
                        Some(PreviewSpanKind::Heading(heading_level(level)))
                    }
                    Tag::Strong => Some(PreviewSpanKind::Bold),
                    Tag::Emphasis => Some(PreviewSpanKind::Italic),
                    Tag::Strikethrough => Some(PreviewSpanKind::Strikethrough),
                    Tag::CodeBlock(_) => Some(PreviewSpanKind::CodeBlock),
                    Tag::BlockQuote(_) => Some(PreviewSpanKind::BlockQuote),
                    Tag::Link { dest_url, .. } if is_safe_href(&dest_url) => {
                        Some(PreviewSpanKind::Link)
                    }
                    _ => None,
                }
            }
            Event::Code(_) => Some(PreviewSpanKind::Code),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                None
            }
            _ => None,
        };
        if let Some(kind) = kind.filter(|_| !range.is_empty()) {
            result.push((kind, range));
            if result.len() > MAX_PREVIEW_SPANS {
                return None;
            }
        }
    }

    Some(result)
}

fn append_spoiler_and_mention_spans(
    original: &str,
    parse_source: &str,
    bindings: &[ComposerMentionBinding],
    custom_emojis: &[CustomEmojiRef],
    spans: &mut Vec<(PreviewSpanKind, Range<usize>)>,
) -> bool {
    let mut in_ignored = 0usize;
    let mut ignored_ranges: Vec<Range<usize>> = Vec::new();
    let mut open_spoilers: Vec<Vec<usize>> = vec![Vec::new()];
    let mut contiguous_mentions: Option<Range<usize>> = None;

    let flush_mentions = |pending: &mut Option<Range<usize>>,
                          out: &mut Vec<(PreviewSpanKind, Range<usize>)>|
     -> bool {
        if let Some(range) = pending.take() {
            return scan_mentions_in_range(parse_source, range, custom_emojis, out);
        }
        true
    };

    for (event, range) in Parser::new_ext(parse_source, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. }) => {
                if !flush_mentions(&mut contiguous_mentions, spans) {
                    return false;
                }
                if in_ignored == 0 {
                    ignored_ranges.push(range);
                }
                in_ignored += 1;
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Link | TagEnd::Image) => {
                if !flush_mentions(&mut contiguous_mentions, spans) {
                    return false;
                }
                in_ignored = in_ignored.saturating_sub(1);
            }
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::TableCell | Tag::Item) => {
                if !flush_mentions(&mut contiguous_mentions, spans) {
                    return false;
                }
                open_spoilers.push(Vec::new());
            }
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell | TagEnd::Item,
            ) => {
                if !flush_mentions(&mut contiguous_mentions, spans) {
                    return false;
                }
                open_spoilers.pop();
            }
            Event::Text(text) if in_ignored == 0 => {
                let leading_escaped = text.starts_with('|')
                    && odd_trailing_backslashes(&parse_source.as_bytes()[..range.start]);
                let spoiler_start = range.start + usize::from(leading_escaped);
                if spoiler_start < range.end
                    && text[usize::from(leading_escaped)..].contains("||")
                    && let Some(stack) = open_spoilers.last_mut()
                    && !scan_spoiler_delims_in_range(
                        parse_source,
                        &(spoiler_start..range.end),
                        stack,
                        spans,
                    )
                {
                    return false;
                }

                match &mut contiguous_mentions {
                    Some(prev) if prev.end == range.start => prev.end = range.end,
                    _ => {
                        if !flush_mentions(&mut contiguous_mentions, spans) {
                            return false;
                        }
                        contiguous_mentions = Some(range);
                    }
                }
            }
            Event::Code(_) => {
                if !flush_mentions(&mut contiguous_mentions, spans) {
                    return false;
                }
                if in_ignored == 0 {
                    ignored_ranges.push(range);
                }
            }
            Event::SoftBreak | Event::HardBreak | Event::Html(_) | Event::InlineHtml(_)
                if !flush_mentions(&mut contiguous_mentions, spans) =>
            {
                return false;
            }
            _ => {}
        }
    }
    flush_mentions(&mut contiguous_mentions, spans)
        && emit_bound_mentions(original, bindings, &ignored_ranges, spans)
}

fn emit_bound_mentions(
    original: &str,
    bindings: &[ComposerMentionBinding],
    ignored_ranges: &[Range<usize>],
    spans: &mut Vec<(PreviewSpanKind, Range<usize>)>,
) -> bool {
    for binding in bindings {
        if original.get(binding.byte_range.clone()) == Some(binding.text.as_str())
            && !overlaps_any(ignored_ranges, &binding.byte_range)
        {
            if spans.len() >= MAX_PREVIEW_SPANS {
                return false;
            }
            spans.push((
                PreviewSpanKind::Mention(MentionTarget::User(binding.id)),
                binding.byte_range.clone(),
            ));
        }
    }
    true
}

fn scan_spoiler_delims_in_range(
    source: &str,
    range: &Range<usize>,
    stack: &mut Vec<usize>,
    spans: &mut Vec<(PreviewSpanKind, Range<usize>)>,
) -> bool {
    let slice = &source[range.clone()];
    let mut offset = 0usize;
    while let Some(found) = slice[offset..].find("||") {
        let abs_start = range.start + offset + found;
        if odd_trailing_backslashes(&source.as_bytes()[..abs_start]) {
            offset += found + 1;
            continue;
        }
        let abs_end = abs_start + 2;
        if let Some(open_start) = stack.last().copied() {
            if open_start + 2 < abs_start {
                if spans.len() >= MAX_PREVIEW_SPANS {
                    return false;
                }
                stack.pop();
                spans.push((PreviewSpanKind::Spoiler, open_start..abs_end));
            } else {
                stack.pop();
            }
        } else if stack.len() < MAX_NESTING_DEPTH {
            stack.push(abs_start);
        }
        offset += found + 2;
    }
    true
}

fn scan_mentions_in_range(
    source: &str,
    range: Range<usize>,
    custom_emojis: &[CustomEmojiRef],
    spans: &mut Vec<(PreviewSpanKind, Range<usize>)>,
) -> bool {
    let slice = &source[range.clone()];
    let bytes = slice.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let abs = range.start + i;
        if bytes[i] == b'@' {
            if let Some((id, end)) = papo_sdk::mentions::parse_user_mention_at(source, abs)
                && end <= range.end
            {
                if spans.len() >= MAX_PREVIEW_SPANS {
                    return false;
                }
                spans.push((PreviewSpanKind::Mention(MentionTarget::User(id)), abs..end));
                i = end - range.start;
                continue;
            }
            if let Some(end) = papo_sdk::mentions::parse_everyone_at(source, abs, true)
                && end <= range.end
            {
                if spans.len() >= MAX_PREVIEW_SPANS {
                    return false;
                }
                spans.push((PreviewSpanKind::Mention(MentionTarget::Everyone), abs..end));
                i = end - range.start;
                continue;
            }
        } else if bytes[i] == b':'
            && !custom_emojis.is_empty()
            && let Some((emoji, end)) = match_custom_emoji_at(source, abs, custom_emojis)
            && end <= range.end
        {
            if spans.len() >= MAX_PREVIEW_SPANS {
                return false;
            }
            spans.push((PreviewSpanKind::CustomEmoji(emoji.id), abs..end));
            i = end - range.start;
            continue;
        }
        i += 1;
    }
    true
}

fn overlaps_any(ranges: &[Range<usize>], target: &Range<usize>) -> bool {
    ranges
        .iter()
        .any(|existing| existing.start < target.end && target.start < existing.end)
}

fn mask_bindings<'a>(source: &'a str, bindings: &[ComposerMentionBinding]) -> Cow<'a, str> {
    if bindings.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut ranges: Vec<Range<usize>> = bindings
        .iter()
        .filter(|b| {
            !b.byte_range.is_empty() && source.get(b.byte_range.clone()) == Some(b.text.as_str())
        })
        .map(|b| b.byte_range.clone())
        .collect();
    if ranges.is_empty() {
        return Cow::Borrowed(source);
    }
    ranges.sort_unstable_by_key(|r| (r.start, r.end));

    let mut masked = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for range in ranges {
        let start = range.start.max(cursor);
        if start > cursor {
            masked.push_str(&source[cursor..start]);
        }
        if start < range.end {
            for ch in source[start..range.end].chars() {
                if matches!(ch, '\n' | '\r') {
                    masked.push(ch);
                } else {
                    for _ in 0..ch.len_utf8() {
                        masked.push('x');
                    }
                }
            }
            cursor = range.end;
        }
    }
    if cursor < source.len() {
        masked.push_str(&source[cursor..]);
    }
    Cow::Owned(masked)
}

pub(crate) fn build_utf16_offset_map(source: &str) -> Vec<usize> {
    let mut map = vec![0usize; source.len() + 1];
    let mut utf16 = 0usize;
    for (byte, ch) in source.char_indices() {
        map[byte] = utf16;
        utf16 += ch.len_utf16();
    }
    map[source.len()] = utf16;
    map
}
