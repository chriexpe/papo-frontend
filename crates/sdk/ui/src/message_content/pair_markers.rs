//! Markdown and bracket delimiter classification for composer pair closing.

/// Delimiter kind recognized by composer pair closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Marker {
    Fence,
    Code,
    Bold,
    Underline,
    Strike,
    Spoiler,
    Italic,
    ItalicUnderscore,
    Paren,
    Bracket,
    Brace,
}

impl Marker {
    pub(super) const fn token(self) -> &'static str {
        match self {
            Self::Fence => "```",
            Self::Code => "`",
            Self::Bold => "**",
            Self::Underline => "__",
            Self::Strike => "~~",
            Self::Spoiler => "||",
            Self::Italic => "*",
            Self::ItalicUnderscore => "_",
            Self::Paren => "(",
            Self::Bracket => "[",
            Self::Brace => "{",
        }
    }

    pub(super) const fn closer(self) -> &'static str {
        match self {
            Self::Fence => "\n```",
            Self::Code => "`",
            Self::Bold => "**",
            Self::Underline => "__",
            Self::Strike => "~~",
            Self::Spoiler => "||",
            Self::Italic => "*",
            Self::ItalicUnderscore => "_",
            Self::Paren => ")",
            Self::Bracket => "]",
            Self::Brace => "}",
        }
    }

    pub(super) const fn is_bracket(self) -> bool {
        matches!(self, Self::Paren | Self::Bracket | Self::Brace)
    }

    const ALL_MD: [Self; 8] = [
        Self::Fence,
        Self::Bold,
        Self::Underline,
        Self::Strike,
        Self::Spoiler,
        Self::Code,
        Self::Italic,
        Self::ItalicUnderscore,
    ];
}

pub(super) const fn auto_closes(marker: Marker) -> bool {
    matches!(
        marker,
        Marker::Fence
            | Marker::Code
            | Marker::Bold
            | Marker::Underline
            | Marker::Strike
            | Marker::Spoiler
    )
}

pub(super) const fn bracket_from_char(ch: char) -> Option<Marker> {
    match ch {
        '(' => Some(Marker::Paren),
        '[' => Some(Marker::Bracket),
        '{' => Some(Marker::Brace),
        _ => None,
    }
}

fn starts_with_at(chars: &[char], at: usize, token: &str) -> bool {
    token
        .chars()
        .enumerate()
        .all(|(offset, ch)| chars.get(at + offset) == Some(&ch))
}

#[derive(Clone, Copy)]
pub(super) enum TextDelta {
    Insert { index: usize, ch: char },
    Delete { index: usize },
    Other,
}

pub(super) fn classify(prev: &[char], text: &[char], caret: usize) -> TextDelta {
    if text.len() == prev.len() + 1
        && caret >= 1
        && caret <= text.len()
        && prev[..caret - 1] == text[..caret - 1]
        && prev[caret - 1..] == text[caret..]
    {
        return TextDelta::Insert {
            index: caret - 1,
            ch: text[caret - 1],
        };
    }
    if prev.len() == text.len() + 1
        && caret <= text.len()
        && prev[..caret] == text[..caret]
        && prev[caret + 1..] == text[caret..]
    {
        return TextDelta::Delete { index: caret };
    }
    TextDelta::Other
}

/// Computes the stack of open Markdown delimiters strictly before the caret.
pub(super) fn open_stack(prefix: &[char]) -> Vec<Marker> {
    let mut stack: Vec<Marker> = Vec::new();
    let mut index = 0;
    while index < prefix.len() {
        let ch = prefix[index];
        if stack.last() == Some(&Marker::Fence) {
            if starts_with_at(prefix, index, "```") {
                stack.pop();
                index += 3;
            } else {
                index += 1;
            }
            continue;
        }
        if ch == '\n' {
            stack.retain(|marker| *marker == Marker::Fence);
            index += 1;
            continue;
        }
        if stack.last() == Some(&Marker::Code) {
            if ch == '`' {
                stack.pop();
            }
            index += 1;
            continue;
        }

        let Some(marker) = Marker::ALL_MD
            .into_iter()
            .find(|candidate| starts_with_at(prefix, index, candidate.token()))
        else {
            index += 1;
            continue;
        };
        let len = marker.token().chars().count();
        let before = index.checked_sub(1).map(|at| prefix[at]);
        let after = prefix.get(index + len).copied();
        let closes =
            stack.last() == Some(&marker) && before.is_some_and(|prev| !prev.is_whitespace());
        if closes {
            stack.pop();
        } else {
            let single = matches!(marker, Marker::Italic | Marker::ItalicUnderscore);
            let boundary = before.is_none_or(|prev| !prev.is_alphanumeric());
            let tight = after.is_none_or(|next| !next.is_whitespace());
            let opens = match marker {
                Marker::Fence => true,
                _ if single => boundary && tight,
                _ => tight,
            };
            if opens {
                stack.push(marker);
            }
        }
        index += len;
    }
    stack
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PairFrame {
    pub(super) kind: Marker,
    pub(super) opener: Vec<char>,
    pub(super) closer: Vec<char>,
    pub(super) start: usize,
    pub(super) at: usize,
}

impl PairFrame {
    pub(super) const fn is_empty(&self) -> bool {
        self.at == self.start + self.opener.len()
    }

    pub(super) const fn is_fence(&self) -> bool {
        matches!(self.kind, Marker::Fence)
    }

    pub(super) const fn appends_space_on_exit(&self) -> bool {
        !self.is_fence() && !self.kind.is_bracket()
    }
}

pub(super) fn frame_intact(text: &[char], frame: &PairFrame) -> bool {
    text.get(frame.start..frame.start + frame.opener.len()) == Some(frame.opener.as_slice())
        && text.get(frame.at..frame.at + frame.closer.len()) == Some(frame.closer.as_slice())
}

pub(super) fn rebase_frames(
    previous: &[char],
    text: &[char],
    frames: Vec<PairFrame>,
) -> Vec<PairFrame> {
    let prefix = previous
        .iter()
        .zip(text)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = previous[prefix..]
        .iter()
        .rev()
        .zip(text[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = previous.len() - suffix;
    let delta = text.len().cast_signed() - previous.len().cast_signed();
    frames
        .into_iter()
        .filter_map(|mut active| {
            let intact = if old_end <= active.start {
                active.start = active.start.saturating_add_signed(delta);
                active.at = active.at.saturating_add_signed(delta);
                true
            } else if prefix >= active.start + active.opener.len() && old_end <= active.at {
                active.at = active.at.saturating_add_signed(delta);
                true
            } else {
                prefix >= active.at + active.closer.len()
            };
            (intact && frame_intact(text, &active)).then_some(active)
        })
        .collect()
}
