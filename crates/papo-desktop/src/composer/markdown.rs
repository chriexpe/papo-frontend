//! Markdown in place (`docs/design.md` §3, "Markdown in place"): the SDK's
//! preview spans (`preview_spans_with_bindings`) drawn over the field as
//! Rune spans and strike lines. The content takes its style and the markers
//! stay visible, dimmed to `text.muted`.
//!
//! Pure. The SDK's spans cover their markers (`**oi**`, `## Title`); finding
//! the markers inside each one is this module's job. Every range keeps
//! source offsets and no style changes the text's size, so nothing moves
//! as a style applies.

use std::ops::Range;

use papo_sdk_ui::message_content::{PreviewSpan, PreviewSpanKind};
use rune::{Color, Span};

use crate::design::{
    CHIP_PAD_PT, LINK_UNDERLINE_ALPHA, Palette, SPOILER_PAD_PT, WEIGHT_MEDIUM, WEIGHT_SEMIBOLD,
};

/// What the field draws over its text.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct FieldStyle {
    /// Styled byte ranges, content first and markers after, so a marker's
    /// dimming wins where they overlap.
    pub(crate) spans: Vec<Span>,
    /// Strikethrough lines over byte ranges, in the text's colour.
    pub(crate) strikes: Vec<(usize, usize, Option<Color>)>,
}

/// The style of `source`, whose preview spans are `preview`.
pub(crate) fn field_style(source: &str, preview: &[PreviewSpan], palette: &Palette) -> FieldStyle {
    let mut style = FieldStyle::default();
    let mut markers = Vec::new();
    for span in preview {
        let Some(parts) = parts(source, span.kind, span.byte_range.clone()) else {
            continue;
        };
        let content = parts.content;
        match span.kind {
            PreviewSpanKind::Bold | PreviewSpanKind::Heading(_) => {
                style
                    .spans
                    .push(content_span(&content).weight(WEIGHT_SEMIBOLD));
            }
            PreviewSpanKind::Italic => style.spans.push(content_span(&content).italic()),
            PreviewSpanKind::Strikethrough => {
                if content.start < content.end {
                    style.strikes.push((content.start, content.end, None));
                }
            }
            PreviewSpanKind::Code => style.spans.push(
                content_span(&content)
                    .mono()
                    .bg(palette.surface_code, SPOILER_PAD_PT),
            ),
            PreviewSpanKind::CodeBlock => style.spans.push(content_span(&content).mono()),
            PreviewSpanKind::Spoiler => style
                .spans
                .push(content_span(&content).bg(palette.surface_hover, SPOILER_PAD_PT)),
            PreviewSpanKind::Link => style.spans.push(
                content_span(&content)
                    .color(palette.link)
                    .underline(palette.link.alpha(LINK_UNDERLINE_ALPHA)),
            ),
            PreviewSpanKind::BlockQuote => {
                style
                    .spans
                    .push(content_span(&content).color(palette.text_secondary));
            }
            PreviewSpanKind::Mention(_) => style.spans.push(
                content_span(&content)
                    .weight(WEIGHT_MEDIUM)
                    .color(palette.link)
                    .bg(palette.accent_soft, CHIP_PAD_PT),
            ),
        }
        markers.extend(parts.markers);
    }
    style.spans.extend(
        markers
            .into_iter()
            .filter(|marker| marker.start < marker.end)
            .map(|marker| Span::new(marker.start, marker.end).color(palette.text_muted)),
    );
    style
}

fn content_span(content: &Range<usize>) -> Span {
    Span::new(content.start, content.end)
}

/// One marker, to which a span may add more.
fn marker(range: Range<usize>) -> Vec<Range<usize>> {
    let mut markers = Vec::with_capacity(2);
    markers.push(range);
    markers
}

/// A preview span split into the text that takes the style and the
/// markers around or inside it.
#[derive(Debug, PartialEq)]
struct Parts {
    content: Range<usize>,
    markers: Vec<Range<usize>>,
}

impl Parts {
    /// The whole span is content: no markers found.
    fn plain(range: Range<usize>) -> Parts {
        Parts {
            content: range,
            markers: Vec::new(),
        }
    }
}

/// Splits the span `range` of `kind` in `source`; none when the SDK's range
/// is not on `source`'s character boundaries.
fn parts(source: &str, kind: PreviewSpanKind, range: Range<usize>) -> Option<Parts> {
    let text = source.get(range.clone())?;
    let parts = match kind {
        PreviewSpanKind::Bold | PreviewSpanKind::Spoiler => around(range, text, 2),
        PreviewSpanKind::Italic => around(range, text, 1),
        PreviewSpanKind::Strikethrough => around(range, text, edge_run(text, '~')),
        PreviewSpanKind::Code => around(range, text, edge_run(text, '`')),
        PreviewSpanKind::Heading(_) => heading(range, text),
        PreviewSpanKind::Link => link(range, text),
        PreviewSpanKind::BlockQuote => quote(range, text),
        PreviewSpanKind::CodeBlock => code_block(range, text),
        PreviewSpanKind::Mention(_) => Parts::plain(range),
    };
    Some(parts)
}

/// The delimiters a symmetric span opens and closes with.
const DELIMITERS: &[u8] = b"*_~|`";

/// `width` delimiter bytes at each end of `text` are markers.
fn around(range: Range<usize>, text: &str, width: usize) -> Parts {
    let bytes = text.as_bytes();
    let is_delimiter = |byte: &u8| DELIMITERS.contains(byte);
    if width == 0
        || bytes.len() < width * 2
        || !bytes[..width].iter().all(is_delimiter)
        || !bytes[bytes.len() - width..].iter().all(is_delimiter)
    {
        return Parts::plain(range);
    }
    let (start, end) = (range.start, range.end);
    Parts {
        content: start + width..end - width,
        markers: vec![start..start + width, end - width..end],
    }
}

/// How many `ch` open `text` and as many close it: `~~a~~` is 2, `~a~` 1.
fn edge_run(text: &str, ch: char) -> usize {
    let leading = text.chars().take_while(|&c| c == ch).count();
    let trailing = text.chars().rev().take_while(|&c| c == ch).count();
    leading.min(trailing)
}

/// `## Title`: the hashes and the space after them. A setext heading (a
/// line of `=` under it) has no marker to dim in its first line.
fn heading(range: Range<usize>, text: &str) -> Parts {
    let hashes = text.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 {
        return Parts::plain(range);
    }
    let gap = text[hashes..]
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    let marker_end = range.start + hashes + gap;
    Parts {
        content: marker_end..range.end,
        markers: marker(range.start..marker_end),
    }
}

/// `[label](url)`: the brackets and the destination are markers, the label
/// the link; `<url>`: the angle brackets.
fn link(range: Range<usize>, text: &str) -> Parts {
    let (start, end) = (range.start, range.end);
    if text.starts_with('[') && text.ends_with(')') {
        let Some(split) = text.rfind("](") else {
            return Parts::plain(range);
        };
        return Parts {
            content: start + 1..start + split,
            markers: vec![start..start + 1, start + split..end],
        };
    }
    if text.len() >= 2 && text.starts_with('<') && text.ends_with('>') {
        return Parts {
            content: start + 1..end - 1,
            markers: vec![start..start + 1, end - 1..end],
        };
    }
    Parts::plain(range)
}

/// `> quoted`: each line's `>` (nested ones too) and the space after it.
fn quote(range: Range<usize>, text: &str) -> Parts {
    let mut markers = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let bytes = line.as_bytes();
        let mut at = bytes.iter().take(3).take_while(|&&b| b == b' ').count();
        while bytes.get(at) == Some(&b'>') {
            let marker_start = at;
            at += 1;
            if bytes.get(at) == Some(&b' ') {
                at += 1;
            }
            markers.push(range.start + line_start + marker_start..range.start + line_start + at);
            at += bytes[at..]
                .iter()
                .take(3)
                .take_while(|&&b| b == b' ')
                .count();
        }
        line_start += line.len();
    }
    Parts {
        content: range,
        markers,
    }
}

/// A fenced block: its opening line (fence and language) and its closing
/// fence are markers, the code between them the content. An indented block
/// has no markers.
fn code_block(range: Range<usize>, text: &str) -> Parts {
    let fence = if text.starts_with("```") {
        '`'
    } else if text.starts_with("~~~") {
        '~'
    } else {
        return Parts::plain(range);
    };
    let start = range.start;
    let Some(first_break) = text.find('\n') else {
        // Only the opening fence so far.
        return Parts {
            content: range.end..range.end,
            markers: vec![range],
        };
    };
    let mut markers = marker(start..start + first_break);
    let body_start = start + first_break + 1;
    let mut body_end = range.end;
    let trimmed = text.trim_end_matches('\n');
    if let Some(last_break) = trimmed.rfind('\n')
        && last_break >= first_break
    {
        let last_line = &trimmed[last_break + 1..];
        if last_line.trim_start().starts_with(fence) && last_line.trim().chars().all(|c| c == fence)
        {
            body_end = start + last_break;
            markers.push(start + last_break + 1..start + trimmed.len());
        }
    }
    Parts {
        content: body_start..body_end.max(body_start),
        markers,
    }
}

#[cfg(test)]
mod tests;
