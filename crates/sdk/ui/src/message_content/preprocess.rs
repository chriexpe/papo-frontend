//! Message Markdown preprocessing, link sanitization and plain-text fast-path checks.

use std::{borrow::Cow, ops::Range};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use url::Url;

use crate::message_content::model::MAX_NESTING_DEPTH;

pub(crate) const fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        .union(Options::ENABLE_STRIKETHROUGH)
        .union(Options::ENABLE_TASKLISTS)
}

pub(crate) struct CheckedOffsetEvents<I> {
    pub(crate) inner: I,
    pub(crate) depth: usize,
    pub(crate) exceeded: bool,
}

impl<'a, I: Iterator<Item = (Event<'a>, Range<usize>)>> Iterator for CheckedOffsetEvents<I> {
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.exceeded {
            return None;
        }
        let (event, range) = self.inner.next()?;
        match &event {
            Event::Start(_) => {
                if self.depth == MAX_NESTING_DEPTH {
                    self.exceeded = true;
                    return None;
                }
                self.depth += 1;
            }
            Event::End(_) => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        Some((event, range))
    }
}

pub(crate) fn odd_trailing_backslashes(prefix: &[u8]) -> bool {
    let backslashes = prefix
        .iter()
        .rev()
        .take_while(|&&byte| byte == b'\\')
        .count();
    backslashes % 2 == 1
}

pub(crate) const fn is_inline_end(end: TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image
    )
}

pub(crate) const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

const BLANK_LINE_PREFIX: &str = "PAPOMESSAGEBLANKLINETOKEN";

/// Normalizes CRLF and lone CR sequences to `\n`.
pub(crate) fn normalize_newlines(content: &str) -> Cow<'_, str> {
    let Some(first) = content.find('\r') else {
        return Cow::Borrowed(content);
    };
    let mut out = String::with_capacity(content.len());
    out.push_str(&content[..first]);
    let mut rest = &content[first..];
    while let Some(at) = rest.find('\r') {
        out.push_str(&rest[..at]);
        out.push('\n');
        rest = &rest[at + 1..];
        if let Some(after) = rest.strip_prefix('\n') {
            rest = after;
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Inserts collision-free spacer markers for extra blank lines outside code blocks and lists.
pub(crate) fn preserve_blank_lines(content: &str) -> (Cow<'_, str>, String) {
    if !content.contains("\n\n") {
        return (Cow::Borrowed(content), String::new());
    }
    let mut marker = String::from(BLANK_LINE_PREFIX);
    while content.contains(&marker) {
        marker.push('_');
    }

    let protected = protected_blank_ranges(content);
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    for range in protected {
        if range.start > cursor {
            push_blank_segment(&mut out, &content[cursor..range.start], &marker);
        }
        if range.end > cursor {
            let start = range.start.max(cursor);
            out.push_str(&content[start..range.end]);
            cursor = range.end;
        }
    }
    if cursor < content.len() {
        push_blank_segment(&mut out, &content[cursor..], &marker);
    }
    (Cow::Owned(out), marker)
}

fn protected_blank_ranges(content: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut depth = 0usize;
    let mut active_start: Option<(usize, usize)> = None;
    for (event, range) in Parser::new_ext(content, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                depth += 1;
                if active_start.is_none()
                    && matches!(
                        tag,
                        Tag::CodeBlock(_) | Tag::List(_) | Tag::HtmlBlock | Tag::Table(_)
                    )
                {
                    active_start = Some((depth, range.start));
                }
            }
            Event::End(_) => {
                if let Some((start_depth, start)) = active_start
                    && depth == start_depth
                {
                    active_start = None;
                    let mut end = range.end.min(content.len());
                    let trimmed = start + content[start..end].trim_end_matches('\n').len();
                    if !content[trimmed..].trim_matches('\n').is_empty() {
                        end = trimmed;
                    }
                    if start < end {
                        ranges.push(start..end);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    ranges
}

fn push_blank_segment(out: &mut String, segment: &str, marker: &str) {
    let mut copied = 0;
    let mut scanned = 0;
    while let Some(relative) = segment[scanned..].find('\n') {
        let at = scanned + relative;
        scanned = at + 1;
        if segment.as_bytes().get(scanned) != Some(&b'\n') {
            continue;
        }
        out.push_str(&segment[copied..at]);
        let run = segment.as_bytes()[at..]
            .iter()
            .take_while(|&&byte| byte == b'\n')
            .count();
        out.push_str("\n\n");
        for index in 0..run - 1 {
            if index > 0 {
                out.push_str("\n\n");
            }
            out.push_str(marker);
        }
        out.push_str("\n\n");
        scanned = at + run;
        copied = scanned;
    }
    out.push_str(&segment[copied..]);
}

/// Keeps leading `+` literal while preserving `-`, `*` and ordered list markers and skipping code.
pub(crate) fn escape_plus_list_markers(source: &str, options: Options) -> Cow<'_, str> {
    if !source.contains('+') {
        return Cow::Borrowed(source);
    }
    let mut markers = Parser::new_ext(source, options)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            (matches!(event, Event::Start(Tag::Item))
                && source.as_bytes().get(range.start) == Some(&b'+'))
            .then_some(range.start)
        })
        .peekable();
    if markers.peek().is_none() {
        return Cow::Borrowed(source);
    }
    let mut escaped = String::with_capacity(source.len());
    let mut copied = 0;
    for at in markers {
        escaped.push_str(&source[copied..at]);
        escaped.push('\\');
        copied = at;
    }
    escaped.push_str(&source[copied..]);
    Cow::Owned(escaped)
}

/// Returns `true` only for `http`, `https` and `mailto` URLs accepted as message links.
///
/// Rejects `javascript:`, `data:`, `vbscript:`, `file:`, `ftp:`, relative paths and anchors.
#[must_use]
pub fn is_safe_href(href: &str) -> bool {
    let Ok(parsed) = Url::parse(href.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https" | "mailto")
}

/// Conservative check for plain text containing no Markdown delimiters or block prefixes.
///
/// Callers may still need token/emoji scanning even when this returns `true`.
#[must_use]
pub fn is_plain(text: &str) -> bool {
    if text.chars().any(|ch| {
        matches!(
            ch,
            '*' | '_' | '~' | '`' | '\\' | '[' | ']' | '<' | '>' | '&' | '|' | '!' | '\t'
        )
    }) {
        return false;
    }
    text.split('\n').all(|line| {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() >= 4 {
            return false;
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        let ordered = digits > 0 && matches!(trimmed[digits..].chars().next(), Some('.' | ')'));
        !(ordered || trimmed.starts_with(['#', '-', '=', '>']))
    })
}
