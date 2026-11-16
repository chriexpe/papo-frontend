//! Triple-backtick fence normalization preserving inline code spans and alternate fences.
//!
//! Leaves `CommonMark`-valid tilde fences, 4+ backtick fences, indented code blocks, properly
//! closed fenced blocks and non-triple inline code spans intact; normalizes only glued or
//! unclosed triple-backtick pairs into clean line-delimited fences.

use std::{borrow::Cow, ops::Range};

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::message_content::{
    model::Block,
    preprocess::{markdown_options, normalize_newlines, odd_trailing_backslashes},
};

/// Ensures triple-backtick code fences always open/close on their own lines, even when glued
/// directly against adjacent text (`antes```x```depois` or ```` ```{"ok":true}``` ````).
#[must_use]
pub fn normalize_fenced_code_blocks(content: &str) -> String {
    normalize_fences(content).into_owned()
}

pub(crate) fn normalize_fences(content: &str) -> Cow<'_, str> {
    if !content.contains("```") {
        return Cow::Borrowed(content);
    }
    let protected = protected_fence_ranges(content);
    let Some(first) = find_fence_outside(content, 0, &protected) else {
        return Cow::Borrowed(content);
    };
    if find_fence_outside(content, first + 3, &protected).is_none() {
        return Cow::Borrowed(content);
    }
    let mut out = String::with_capacity(content.len() + 16);
    let mut rest_start = 0;
    while let Some(open) = find_fence_outside(content, rest_start, &protected) {
        let inner_start = open + 3;
        let Some(close) = find_fence_outside(content, inner_start, &protected) else {
            break;
        };
        let inner = normalize_newlines(&content[inner_start..close]);
        let mut info = "";
        let mut body: &str = &inner;

        if let Some(stripped) = body.strip_prefix('\n') {
            body = stripped;
        } else if let Some(first_break) = body.find('\n') {
            let possible = body[..first_break].trim();
            if is_info_string(possible) {
                info = possible;
                body = &body[first_break + 1..];
            }
        }
        if let Some(stripped) = body.strip_suffix('\n') {
            body = stripped;
        }

        out.push_str(&content[rest_start..open]);
        let before_needs_break = open > 0 && content.as_bytes()[open - 1] != b'\n';
        let after = close + 3;
        let after_needs_break = after < content.len() && content.as_bytes()[after] != b'\n';
        if before_needs_break {
            out.push('\n');
        }
        out.push_str("```");
        out.push_str(info);
        out.push('\n');
        out.push_str(body);
        out.push_str("\n```");
        if after_needs_break {
            out.push('\n');
        }
        rest_start = after;
    }
    out.push_str(&content[rest_start..]);
    Cow::Owned(out)
}

pub(crate) fn read_code_block<'e>(
    kind: CodeBlockKind<'e>,
    events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>,
) -> Block {
    let info = match kind {
        CodeBlockKind::Fenced(raw_info) => {
            let trimmed = raw_info.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CodeBlockKind::Indented => None,
    };
    let mut text = String::new();
    for (code_event, _) in events.by_ref() {
        match code_event {
            Event::Text(chunk) | Event::Code(chunk) => text.push_str(&chunk),
            Event::End(_) => break,
            _ => {}
        }
    }
    if let Some(stripped) = text.strip_suffix('\n') {
        text.truncate(stripped.len());
    }
    Block::CodeBlock { info, text }
}

fn protected_fence_ranges(content: &str) -> Vec<Range<usize>> {
    let mut protected = Vec::new();
    let mut active: Option<(CodeBlockKind<'_>, usize)> = None;
    for (event, range) in Parser::new_ext(content, markdown_options()).into_offset_iter() {
        match event {
            Event::Code(_) => {
                // Only protect inline spans delimited by != 3 backticks so glued ```x``` spans still normalize.
                let backticks = content.as_bytes()[range.start..]
                    .iter()
                    .take_while(|&&b| b == b'`')
                    .count();
                if backticks != 3 {
                    protected.push(range);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                active = Some((kind, range.start));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((kind, start)) = active.take()
                    && should_protect_code_block(content, &kind, start..range.end)
                {
                    protected.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    protected
}

fn should_protect_code_block(content: &str, kind: &CodeBlockKind<'_>, range: Range<usize>) -> bool {
    if matches!(kind, CodeBlockKind::Indented) {
        return true;
    }
    let slice = &content[range];
    let opener = slice.trim_start_matches(' ');
    if opener.starts_with('~') || opener.bytes().take_while(|&b| b == b'`').count() >= 4 {
        return true;
    }
    let Some(first_nl) = slice.find('\n') else {
        return false;
    };
    let trimmed = slice.trim_end_matches(['\r', '\n', ' ', '\t']);
    if trimmed.len() <= first_nl {
        return false;
    }
    let mut last_line = trimmed[first_nl + 1..]
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    while let Some(rest) = last_line.strip_prefix('>') {
        last_line = rest.strip_prefix(' ').unwrap_or(rest);
    }
    last_line = last_line.trim_start_matches(' ');
    last_line.starts_with("```") && last_line[3..].bytes().all(|b| b == b'`')
}

fn find_fence_outside(content: &str, from: usize, protected: &[Range<usize>]) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut cursor = from;
    while let Some(relative) = content[cursor..].find("```") {
        let at = cursor + relative;
        let run = bytes[at..].iter().take_while(|&&b| b == b'`').count();
        let preceded_by_backtick = at > 0 && bytes[at - 1] == b'`';
        if !preceded_by_backtick
            && run == 3
            && !odd_trailing_backslashes(&bytes[..at])
            && !overlaps_protected(at..at + 3, protected)
        {
            return Some(at);
        }
        cursor = at + run;
    }
    None
}

fn overlaps_protected(span: Range<usize>, protected: &[Range<usize>]) -> bool {
    protected
        .iter()
        .any(|range| span.start < range.end && span.end > range.start)
}

fn is_info_string(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '+' | '-' | ',' | '#'))
}
