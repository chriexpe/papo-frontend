//! Canonical mention token syntax, encoding, decoding and context-free scanning.
//!
//! Verified against pinned backend commit `9da01ccc191a764517441f7dde743beb747ea516`
//! (`backend/internal/services/notifications.go`, `services/search.go`, `openapi.yml`):
//! user mentions use `@mention(<@uuid>)` (`(?i)@mention\(<@([0-9a-f-...])>\)`) and broadcast mentions
//! use `@everyone` (`(?i)@everyone\b`). Notification classification scans raw message content
//! context-free (including inside code spans and fenced code blocks); Markdown-aware placement
//! belongs in `papo-sdk-ui`. No role or channel mention tokens exist in the pinned backend.

use std::ops::Range;

use papo_api::Uuid;

const USER_PREFIX: &str = "@mention(<@";
const USER_SUFFIX: &str = ">)";
const UUID_HEX_LEN: usize = 36;
const EVERYONE: &str = "@everyone";

/// Canonical mention target referenced inside a message body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MentionTarget {
    /// Direct user mention (`@mention(<@uuid>)`).
    User(Uuid),
    /// Broadcast mention (`@everyone`).
    Everyone,
}

/// Byte span and target of a canonical mention token recognized in source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MentionToken {
    /// Canonical target carried by the token.
    pub target: MentionTarget,
    /// Inclusive UTF-8 start byte index in the source string.
    pub start: usize,
    /// Exclusive UTF-8 end byte index in the source string.
    pub end: usize,
}

impl MentionToken {
    /// Byte range (`start..end`) of this token in the source string.
    #[must_use]
    pub const fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// One validated editable mention span bound to a canonical user ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionBinding {
    /// Canonical backend user UUID.
    pub id: Uuid,
    /// UTF-8 start byte index in the editable display string.
    pub start: usize,
    /// UTF-8 end byte index in the editable display string.
    pub end: usize,
    /// Expected exact slice (`&display[start..end]`, typically `@display_name`).
    pub text: String,
}

/// Result of decoding canonical wire mentions into editable text with tracked byte bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedMentions {
    /// Editable text with known user mentions replaced by `@name` and unknown IDs preserved raw.
    pub text: String,
    /// Exact byte bindings for every replaced mention in `text`.
    pub bindings: Vec<MentionBinding>,
}

/// Formats a user UUID as its canonical backend mention token (`@mention(<@uuid>)`).
#[must_use]
pub fn encode_user_mention(id: Uuid) -> String {
    format!("{USER_PREFIX}{id}{USER_SUFFIX}")
}

/// Parses an exact canonical `@mention(<@uuid>)` token and returns its UUID.
#[must_use]
pub fn parse_user_mention(token: &str) -> Option<Uuid> {
    let (id, end) = parse_user_mention_at(token, 0)?;
    (end == token.len()).then_some(id)
}

/// Parses a canonical `@mention(<@uuid>)` token at UTF-8 byte `start`.
///
/// Follows the backend regex `(?i)@mention\(<@([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})>\)`:
/// case-insensitive ASCII matching, 36-character hyphenated UUID, and exact `>)` closing delimiter.
#[must_use]
pub fn parse_user_mention_at(text: &str, start: usize) -> Option<(Uuid, usize)> {
    let rest = text.get(start..)?;
    let prefix = rest.get(..USER_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(USER_PREFIX) {
        return None;
    }
    let uuid_end = USER_PREFIX.len() + UUID_HEX_LEN;
    let uuid_str = rest.get(USER_PREFIX.len()..uuid_end)?;
    if !is_hyphenated_hex_uuid(uuid_str) {
        return None;
    }
    let suffix_end = uuid_end + USER_SUFFIX.len();
    if rest.get(uuid_end..suffix_end) != Some(USER_SUFFIX) {
        return None;
    }
    let id = Uuid::parse_str(uuid_str).ok()?;
    Some((id, start + suffix_end))
}

/// Recognizes an `@everyone` token at UTF-8 byte `start`, returning its exclusive end byte.
///
/// Backend notification classification checks `(?i)@everyone\b` (`require_leading_boundary = false`).
/// Rich-text UI highlighting sets `require_leading_boundary = true` so substrings inside email
/// addresses or usernames (`foo@everyone`) are not highlighted.
#[must_use]
pub fn parse_everyone_at(
    text: &str,
    start: usize,
    require_leading_boundary: bool,
) -> Option<usize> {
    let rest = text.get(start..)?;
    let prefix = rest.get(..EVERYONE.len())?;
    if !prefix.eq_ignore_ascii_case(EVERYONE) {
        return None;
    }
    if require_leading_boundary
        && let Some(prev) = text[..start].chars().next_back()
        && !is_everyone_leading_boundary(prev)
    {
        return None;
    }
    let end = start + EVERYONE.len();
    if let Some(&next) = text.as_bytes().get(end)
        && is_re2_word_byte(next)
    {
        return None;
    }
    Some(end)
}

/// Scans `content` context-free (including inside code spans/blocks) for canonical mentions.
#[must_use]
pub fn scan_mentions(content: &str) -> Vec<MentionToken> {
    let bytes = content.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if let Some((id, end)) = parse_user_mention_at(content, index) {
            tokens.push(MentionToken {
                target: MentionTarget::User(id),
                start: index,
                end,
            });
            index = end;
            continue;
        }
        if let Some(end) = parse_everyone_at(content, index, false) {
            tokens.push(MentionToken {
                target: MentionTarget::Everyone,
                start: index,
                end,
            });
            index = end;
            continue;
        }
        index += 1;
    }
    tokens
}

/// Returns every canonical user UUID mentioned in `content` using context-free scanning.
#[must_use]
pub fn extract_user_mentions(content: &str) -> Vec<Uuid> {
    scan_mentions(content)
        .into_iter()
        .filter_map(|token| match token.target {
            MentionTarget::User(id) => Some(id),
            MentionTarget::Everyone => None,
        })
        .collect()
}

/// Context-free classification check matching `backend/internal/services/notifications.go`:
/// returns `true` when `content` contains `@mention(<@user>)` anywhere, even inside code.
#[must_use]
pub fn contains_user_mention(content: &str, user: Uuid) -> bool {
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if let Some((id, end)) = parse_user_mention_at(content, index) {
            if id == user {
                return true;
            }
            index = end;
        } else {
            index += 1;
        }
    }
    false
}

/// Context-free classification check matching `backend/internal/services/notifications.go`
/// `(?i)@everyone\b` regex over raw message content.
#[must_use]
pub fn contains_everyone_mention(content: &str) -> bool {
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if parse_everyone_at(content, index, false).is_some() {
            return true;
        }
        index += 1;
    }
    false
}

/// Decodes canonical user mention tokens in `content` into editable `@name` spans using `resolve`.
///
/// If `resolve` returns `None` or an empty string, the original canonical token (`@mention(<@uuid>)`)
/// is preserved verbatim with no substitution or binding.
pub fn decode_mentions_for_editing<'a>(
    content: &str,
    mut resolve: impl FnMut(Uuid) -> Option<&'a str>,
) -> DecodedMentions {
    let bytes = content.as_bytes();
    let mut text = String::with_capacity(content.len());
    let mut bindings = Vec::new();
    let mut cursor = 0;
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if let Some((id, end)) = parse_user_mention_at(content, index) {
            if let Some(name) = resolve(id).filter(|name| !name.is_empty()) {
                text.push_str(&content[cursor..index]);
                let start = text.len();
                text.push('@');
                text.push_str(name);
                let bound_end = text.len();
                bindings.push(MentionBinding {
                    id,
                    start,
                    end: bound_end,
                    text: text[start..bound_end].to_owned(),
                });
                cursor = end;
            }
            index = end;
        } else {
            index += 1;
        }
    }

    text.push_str(&content[cursor..]);
    DecodedMentions { text, bindings }
}

/// Encodes validated editable `@name` bindings back into canonical `@mention(<@uuid>)` tokens.
///
/// Bindings whose UTF-8 range does not match `binding.text` or that overlap a preceding accepted
/// binding are ignored cleanly without mutating unrelated text. Unknown canonical tokens already
/// present in `display` remain untouched.
#[must_use]
pub fn encode_mentions_with_bindings(display: &str, bindings: &[MentionBinding]) -> String {
    let mut valid: Vec<&MentionBinding> = bindings
        .iter()
        .filter(|binding| {
            binding.start < binding.end
                && !binding.text.is_empty()
                && display.get(binding.start..binding.end) == Some(binding.text.as_str())
        })
        .collect();
    if valid.is_empty() {
        return display.to_owned();
    }
    valid.sort_by_key(|binding| (binding.start, binding.end));

    let mut out = String::with_capacity(display.len());
    let mut cursor = 0;
    for binding in valid {
        if binding.start < cursor {
            continue;
        }
        out.push_str(&display[cursor..binding.start]);
        out.push_str(&encode_user_mention(binding.id));
        cursor = binding.end;
    }
    out.push_str(&display[cursor..]);
    out
}

fn is_hyphenated_hex_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != UUID_HEX_LEN {
        return false;
    }
    bytes.iter().enumerate().all(|(idx, &byte)| match idx {
        8 | 13 | 18 | 23 => byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

const fn is_re2_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_everyone_leading_boundary(ch: char) -> bool {
    !ch.is_alphanumeric() && ch != '_' && ch != '<'
}

#[cfg(test)]
mod tests;
