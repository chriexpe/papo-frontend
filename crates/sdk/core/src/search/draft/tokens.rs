//! The search field split into whitespace-separated tokens, quotes grouping words, with their
//! byte ranges; what the tokens mean is `draft`'s.

use std::ops::Range;

/// One token: its byte range of the field, quotes and escapes included, and the byte offset of
/// its first colon outside quotes.
pub(super) struct Token {
    pub(super) range: Range<usize>,
    pub(super) colon: Option<usize>,
}

/// The tokens of `raw`, and whether every quote closed. Quotes group words, a backslash inside
/// quotes escapes the next character.
pub(super) fn tokens(raw: &str) -> (Vec<Token>, bool) {
    let mut tokens = Vec::new();
    let mut current: Option<Token> = None;
    let mut quoted = false;
    let mut escaped = false;
    for (at, ch) in raw.char_indices() {
        if !quoted && !escaped && ch.is_whitespace() {
            tokens.extend(current.take());
            continue;
        }
        let token = current.get_or_insert(Token {
            range: at..at,
            colon: None,
        });
        token.range.end = at + ch.len_utf8();
        if escaped {
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch == ':' && !quoted && token.colon.is_none() {
            token.colon = Some(at);
        }
    }
    tokens.extend(current);
    (tokens, !quoted && !escaped)
}

/// A value or word without its quotes and escapes.
pub(super) fn unquote(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut quoted = false;
    let mut escaped = false;
    for ch in raw.chars() {
        if escaped {
            text.push(ch);
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else {
            text.push(ch);
        }
    }
    text
}
