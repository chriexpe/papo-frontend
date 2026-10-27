//! Style editable Markdown without rewriting source or changing text offsets.
use super::theme::Tokens;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat, text::LayoutJob};
use papo_core::markdown::composer::{Kind, StyledRange};
use std::ops::Range;

const BOLD: usize = 6;
const ITALIC: usize = 7;
const STRIKE: usize = 8;
const CODE: usize = 9;
const CODE_BLOCK: usize = 10;
const LINK: usize = 11;
const QUOTE: usize = 12;
const MARKER: usize = 13;

fn slot(kind: Kind) -> usize {
    match kind {
        Kind::Heading(level) => usize::from(level.saturating_sub(1).min(5)),
        Kind::Bold => BOLD,
        Kind::Italic => ITALIC,
        Kind::Strike => STRIKE,
        Kind::Code => CODE,
        Kind::CodeBlock => CODE_BLOCK,
        Kind::Link => LINK,
        Kind::Quote => QUOTE,
    }
}

fn marker_ranges(source: &str, span: &StyledRange) -> Vec<Range<usize>> {
    let text = &source[span.range.clone()];
    let start = span.range.start;
    let end = span.range.end;
    let mut ranges = Vec::new();
    let mut pair = |count: usize| {
        if text.len() >= count * 2 {
            ranges.extend([start..start + count, end - count..end]);
        }
    };
    match span.kind {
        Kind::Bold | Kind::Strike => pair(2),
        Kind::Italic => pair(1),
        Kind::Code => pair(text.bytes().take_while(|byte| *byte == b'`').count()),
        Kind::Heading(_) => {
            let indent = text.len() - text.trim_start_matches(' ').len();
            let hashes = text[indent..]
                .bytes()
                .take_while(|byte| *byte == b'#')
                .count();
            if hashes > 0 {
                ranges.push(start + indent..start + indent + hashes);
                let suffix = text.trim_end();
                let closing = suffix
                    .bytes()
                    .rev()
                    .take_while(|byte| *byte == b'#')
                    .count();
                if closing > 0
                    && suffix.len() > closing
                    && suffix.as_bytes()[suffix.len() - closing - 1].is_ascii_whitespace()
                {
                    ranges.push(start + suffix.len() - closing..start + suffix.len());
                }
            } else if let Some((line, underline)) = text.rsplit_once('\n') {
                let trimmed = underline.trim();
                if !trimmed.is_empty()
                    && (trimmed.bytes().all(|b| b == b'=') || trimmed.bytes().all(|b| b == b'-'))
                {
                    ranges.push(start + line.len() + 1..end);
                }
            }
        }
        Kind::CodeBlock => {
            let opening = text.split('\n').next().unwrap_or_default();
            let trimmed = opening.trim_start_matches(' ');
            let fence = trimmed.as_bytes().first().copied();
            if let Some(fence @ (b'`' | b'~')) = fence {
                let count = trimmed.bytes().take_while(|b| *b == fence).count();
                if count >= 3 {
                    ranges.push(start..start + opening.len());
                    let trimmed = text.trim_end_matches(['\n', '\r']);
                    if let Some((body, closing)) = trimmed.rsplit_once('\n') {
                        let closing = closing.trim();
                        if closing.len() >= count && closing.bytes().all(|b| b == fence) {
                            ranges.push(start + body.len() + 1..start + trimmed.len());
                        }
                    }
                }
            }
        }
        Kind::Link => {
            if text.starts_with('[') {
                ranges.push(start..start + 1);
                if let Some(at) = text.rfind("](").or_else(|| text.rfind("][")) {
                    ranges.push(start + at..end);
                } else if text.ends_with(']') {
                    ranges.push(end - 1..end);
                }
            } else if text.starts_with('<') && text.ends_with('>') {
                pair(1);
            }
        }
        Kind::Quote => {
            let mut offset = start;
            for line in text.split_inclusive('\n') {
                let indent = line.len() - line.trim_start_matches(' ').len();
                if line.as_bytes().get(indent) == Some(&b'>') {
                    ranges.push(offset + indent..offset + indent + 1);
                }
                offset += line.len();
            }
        }
    }
    ranges
}

fn format(active: &[u16; 14], base: &FontId, color: Color32, t: &Tokens) -> TextFormat {
    let marker = active[MARKER] > 0;
    let code = active[CODE] > 0 || active[CODE_BLOCK] > 0;
    let heading = active[..6].iter().position(|count| *count > 0);
    let mut font = FontId::new(base.size, FontFamily::Name("chat".into()));
    if !marker {
        if code {
            font.family = FontFamily::Monospace;
        } else {
            if active[BOLD] > 0 || heading.is_some() {
                font.family = FontFamily::Name("chat-semibold".into());
            }
            if let Some(level) = heading {
                font.size *= [1.6, 1.4, 1.2, 1.1, 1.0, 1.0][level];
            }
        }
    }
    let color = if marker || active[QUOTE] > 0 {
        t.label_secondary
    } else if active[LINK] > 0 && !code {
        t.accent
    } else {
        color
    };
    TextFormat {
        font_id: font,
        color,
        italics: !marker && !code && active[ITALIC] > 0,
        strikethrough: if !marker && !code && active[STRIKE] > 0 {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        underline: if !marker && !code && active[LINK] > 0 {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        background: if code {
            t.fill_soft
        } else {
            Color32::TRANSPARENT
        },
        ..Default::default()
    }
}

/// Sweep nested style boundaries so work grows with source length and span count,
/// rather than checking every span against every character on each layout pass.
pub(super) fn layout_job(
    source: &str,
    spans: &[StyledRange],
    font: &FontId,
    color: Color32,
    t: &Tokens,
) -> LayoutJob {
    let mut events = Vec::new();
    for span in spans {
        if span.range.start > span.range.end
            || span.range.end > source.len()
            || !source.is_char_boundary(span.range.start)
            || !source.is_char_boundary(span.range.end)
        {
            continue;
        }
        events.push((span.range.start, slot(span.kind), 1i8));
        events.push((span.range.end, slot(span.kind), -1i8));
        for range in marker_ranges(source, span) {
            if !range.is_empty() {
                events.push((range.start, MARKER, 1));
                events.push((range.end, MARKER, -1));
            }
        }
    }
    events.sort_unstable_by_key(|event| event.0);
    let mut job = LayoutJob::default();
    let mut active = [0u16; 14];
    let mut cursor = 0;
    let mut index = 0;
    while index < events.len() {
        let at = events[index].0;
        if at > cursor {
            job.append(&source[cursor..at], 0.0, format(&active, font, color, t));
        }
        // Apply starts and ends together at each boundary; nested adjacent spans
        // can otherwise underflow when the sort puts an end before its start.
        let mut changes = [0i32; 14];
        while index < events.len() && events[index].0 == at {
            let (_, slot, delta) = events[index];
            changes[slot] += i32::from(delta);
            index += 1;
        }
        for (count, delta) in active.iter_mut().zip(changes) {
            *count = (i32::from(*count) + delta).max(0) as u16;
        }
        cursor = at;
    }
    if cursor < source.len() || source.is_empty() {
        job.append(&source[cursor..], 0.0, format(&active, font, color, t));
    }
    job.keep_trailing_whitespace = true;
    job
}

#[cfg(test)]
mod tests {
    use super::super::theme::Appearance;
    use super::*;
    fn job(source: &str) -> LayoutJob {
        let tokens = Tokens::new(Appearance::Dark, None);
        layout_job(
            source,
            &papo_core::markdown::composer::styles(source),
            &FontId::proportional(14.0),
            tokens.label,
            &tokens,
        )
    }
    fn at<'a>(job: &'a LayoutJob, needle: &str) -> &'a TextFormat {
        let offset = job.text.find(needle).unwrap();
        &job.sections
            .iter()
            .find(|section| section.byte_range.start.0 <= offset && offset < section.byte_range.end.0)
            .unwrap()
            .format
    }
    #[test]
    fn preview_preserves_unicode_source_and_styles_content_separately_from_markers() {
        let source = "# Test\n🙂 **bold *italic*** ~~gone~~ `**literal**`  ";
        let job = job(source);
        assert_eq!(job.text, source);
        assert!(at(&job, "Test").font_id.size > 14.0);
        assert_eq!(
            at(&job, "bold").font_id.family,
            FontFamily::Name("chat-semibold".into())
        );
        assert!(at(&job, "italic").italics);
        assert!(at(&job, "gone").strikethrough.width > 0.0);
        assert_eq!(at(&job, "literal").font_id.family, FontFamily::Monospace);
        assert_ne!(at(&job, "#").color, at(&job, "Test").color);
        assert_ne!(at(&job, "**bold").color, at(&job, "bold").color);
    }
    #[test]
    fn incomplete_and_escaped_syntax_stays_literal_and_code_does_not_gain_emphasis() {
        for source in [
            "**unfinished",
            "\\*plain\\*",
            "```\n# literal\n**literal**\n```",
        ] {
            let job = job(source);
            assert_eq!(job.text, source);
            assert!(
                job.sections
                    .iter()
                    .all(|section| section.format.font_id.size == 14.0)
            );
            assert!(
                job.sections
                    .iter()
                    .all(|section| section.format.font_id.family
                        != FontFamily::Name("chat-semibold".into()))
            );
        }
    }
}
