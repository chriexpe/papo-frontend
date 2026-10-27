//! Source-preserving editor styles from the same CommonMark parser as sent messages.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading(u8),
    Bold,
    Italic,
    Strike,
    Code,
    CodeBlock,
    Link,
    Quote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledRange {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// Offsets refer to the original UTF-8 source, including editable delimiters.
/// Do not normalize fences, escapes, blank lines or mentions as the message renderer does:
/// changing source lengths here would move the native editor's cursor/IME composition.
pub fn styles(source: &str) -> Vec<StyledRange> {
    if source.len() > 128 * 1024 {
        return Vec::new();
    }
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut result = Vec::new();
    let mut depth = 0usize;
    for (event, mut range) in Parser::new_ext(source, options).into_offset_iter() {
        let kind = match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > 64 {
                    return Vec::new();
                }
                match tag {
                    Tag::Heading { level, .. } => {
                        while range.end > range.start
                            && matches!(source.as_bytes()[range.end - 1], b'\n' | b'\r')
                        {
                            range.end -= 1;
                        }
                        Some(Kind::Heading(level as u8))
                    }
                    Tag::Strong => Some(Kind::Bold),
                    Tag::Emphasis => Some(Kind::Italic),
                    Tag::Strikethrough => Some(Kind::Strike),
                    Tag::CodeBlock(_) => Some(Kind::CodeBlock),
                    Tag::BlockQuote(_) => Some(Kind::Quote),
                    Tag::Link { dest_url, .. } if super::is_safe_href(&dest_url) => {
                        Some(Kind::Link)
                    }
                    _ => None,
                }
            }
            Event::Code(_) => Some(Kind::Code),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                None
            }
            _ => None,
        };
        if let Some(kind) = kind.filter(|_| !range.is_empty()) {
            result.push(StyledRange { range, kind });
            if result.len() > 2048 {
                return Vec::new();
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_keep_exact_source_ranges_and_nested_commonmark() {
        let source = "# Heading\n🙂 **bold and *italic*** ~~gone~~ `**literal**` [site](https://example.test)";
        let ranges = styles(source);
        let contains = |kind, text| {
            ranges
                .iter()
                .any(|span| span.kind == kind && &source[span.range.clone()] == text)
        };
        assert!(contains(Kind::Heading(1), "# Heading"));
        assert!(contains(Kind::Bold, "**bold and *italic***"));
        assert!(contains(Kind::Italic, "*italic*"));
        assert!(contains(Kind::Strike, "~~gone~~"));
        assert!(contains(Kind::Code, "`**literal**`"));
        assert!(contains(Kind::Link, "[site](https://example.test)"));
        assert_eq!(
            ranges.iter().filter(|span| span.kind == Kind::Bold).count(),
            1
        );
    }

    #[test]
    fn code_is_literal_and_incomplete_or_escaped_markers_do_not_invent_styles() {
        let source = "```text\n# not a heading\n**not bold**\n```\n\n\\*plain\\*\n\n**unfinished";
        let ranges = styles(source);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].kind, Kind::CodeBlock);
        assert!(source[ranges[0].range.clone()].contains("**not bold**"));
        assert!(styles("plain 🙂 text").is_empty());
        assert!(styles("[unsafe](javascript:alert)").is_empty());
    }

    #[test]
    fn untrusted_editor_input_has_bounded_preview_work() {
        assert!(styles(&format!("{}kept", "> ".repeat(2000))).is_empty());
        assert!(styles(&"**many** ".repeat(2050)).is_empty());
        assert!(styles(&"x".repeat(128 * 1024 + 1)).is_empty());
    }
}
