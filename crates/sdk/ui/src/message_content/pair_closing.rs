//! Pure composer pair closing for backticks, brackets and paired Markdown delimiters.
//!
//! Pair closing runs only on direct single-keystroke typing (`TextChangeSource::Typing`) and never
//! auto-inserts closers on clipboard paste or IME composition commits.

use crate::message_content::{
    composer_mentions::utf8_offset_to_utf16,
    pair_markers::{
        Marker, PairFrame, TextDelta, auto_closes, bracket_from_char, classify, frame_intact,
        open_stack, rebase_frames,
    },
};

const MAX_PENDING_PAIRS: usize = 32;

/// Origin of a composer text edit; only direct typing may auto-insert closing delimiters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextChangeSource {
    /// Direct single-character user input or backspace.
    Typing,
    /// Clipboard paste.
    Paste,
    /// Input-method composition change or commit.
    ImeCommit,
    /// Programmatic replacement (draft restore, mention insertion, undo).
    Programmatic,
}

/// Active stack of auto-inserted closing delimiters tracked across composer edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PairClosing {
    frames: Vec<PairFrame>,
}

/// Resulting buffer and caret coordinates emitted by [`on_pair_change`] or [`skip_pair_closer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairEdit {
    /// Updated composer text.
    pub text: String,
    /// Caret offset in UTF-8 bytes.
    pub byte_caret: usize,
    /// Caret offset in Unicode scalar values.
    pub char_caret: usize,
    /// Caret offset in UTF-16 code units.
    pub utf16_caret: usize,
}

impl PairEdit {
    fn from_chars(chars: Vec<char>, char_caret: usize) -> Self {
        let text: String = chars.into_iter().collect();
        let byte_caret = text
            .char_indices()
            .nth(char_caret)
            .map_or(text.len(), |(byte, _)| byte);
        let utf16_caret = utf8_offset_to_utf16(&text, byte_caret).unwrap_or(0);
        Self {
            text,
            byte_caret,
            char_caret,
            utf16_caret,
        }
    }
}

fn sync_pairing(pairing: &mut Option<PairClosing>, mut frames: Vec<PairFrame>) {
    *pairing = (!frames.is_empty()).then(|| {
        if frames.len() > MAX_PENDING_PAIRS {
            let drain = frames.len() - MAX_PENDING_PAIRS;
            frames.drain(..drain);
        }
        PairClosing { frames }
    });
}

fn shift_frames(frames: &mut Vec<PairFrame>, chars: &[char], index: usize, is_insert: bool) {
    let delta: isize = if is_insert { 1 } else { -1 };
    frames.retain_mut(|frame| {
        if (is_insert && index <= frame.at) || (!is_insert && index < frame.at) {
            if (is_insert && index <= frame.start) || (!is_insert && index < frame.start) {
                frame.start = frame.start.saturating_add_signed(delta);
            }
            frame.at = frame.at.saturating_add_signed(delta);
        }
        frame_intact(chars, frame)
    });
}

fn leave_pair(chars: &mut Vec<char>, caret: usize, append_space: bool) -> usize {
    if append_space && chars.get(caret).is_none_or(|ch| !ch.is_whitespace()) {
        chars.insert(caret.min(chars.len()), ' ');
        caret + 1
    } else {
        caret
    }
}

/// Processes a composer text change from `prev` to `text` with UTF-8 byte caret `byte_caret`.
///
/// Only `TextChangeSource::Typing` ever inserts a closing backtick, Markdown delimiter or
/// bracket; `Paste`, `ImeCommit` and `Programmatic` edits rebase or invalidate existing pairs
/// without modifying delimiters.
pub fn on_pair_change(
    prev: &str,
    text: &str,
    byte_caret: usize,
    source: TextChangeSource,
    pairing: &mut Option<PairClosing>,
) -> Option<PairEdit> {
    if prev == text || !text.is_char_boundary(byte_caret) {
        return None;
    }
    if source != TextChangeSource::Typing {
        track_pair_change(prev, text, pairing);
        return None;
    }

    let prev_chars: Vec<char> = prev.chars().collect();
    let chars: Vec<char> = text.chars().collect();
    let caret = text[..byte_caret].chars().count();
    let delta = classify(&prev_chars, &chars, caret);

    if let Some(state) = pairing.take()
        && let Some(edit) =
            handle_active_pair_change(state, delta, &prev_chars, &chars, caret, pairing)
    {
        return Some(edit);
    }

    let TextDelta::Insert { index, ch } = delta else {
        return None;
    };
    if let Some(bracket) = bracket_from_char(ch) {
        return try_open_bracket(chars, caret, bracket, pairing);
    }

    let before = open_stack(&chars[..index]);
    let after = open_stack(&chars[..caret]);
    let marker = *after.last().filter(|marker| {
        (after.len() > before.len() || before.last() != Some(*marker))
            && auto_closes(**marker)
            && chars[..caret].ends_with(&marker.token().chars().collect::<Vec<_>>())
    })?;

    let rest = &chars[caret..];
    if rest
        .iter()
        .take_while(|next| **next != '\n')
        .any(|next| !next.is_whitespace() && !matches!(next, ')' | ']' | '}'))
    {
        return None;
    }
    let opener: Vec<char> = marker.token().chars().collect();
    if rest.starts_with(&opener) {
        return None;
    }
    Some(insert_closer(chars, caret, marker, opener, pairing))
}

fn handle_active_pair_change(
    mut state: PairClosing,
    delta: TextDelta,
    prev_chars: &[char],
    chars: &[char],
    caret: usize,
    pairing: &mut Option<PairClosing>,
) -> Option<PairEdit> {
    let active = state.frames.last().cloned()?;
    match delta {
        TextDelta::Insert { index, ch }
            if index == active.at
                && Some(&ch) == active.closer.first()
                && !(active.is_fence() && ch == '\n') =>
        {
            let mut popped = state.frames.pop()?;
            let add_space = popped.appends_space_on_exit() && !popped.is_empty();
            popped.closer.remove(0);
            popped.at += 1;
            let moved = popped.at;
            let done = popped.closer.is_empty();
            if !done {
                state.frames.push(popped);
            }
            let mut written = prev_chars.to_vec();
            let next_caret = if done
                && add_space
                && !state.frames.iter().any(|outer| outer.at == moved)
                && written.get(moved).is_none_or(|c| !c.is_whitespace())
            {
                let next = leave_pair(&mut written, moved, true);
                for outer in &mut state.frames {
                    if outer.at >= moved {
                        outer.at += 1;
                    }
                }
                next
            } else {
                moved
            };
            state.frames.retain(|frame| frame_intact(&written, frame));
            sync_pairing(pairing, state.frames);
            return Some(PairEdit::from_chars(written, next_caret));
        }
        TextDelta::Insert { index, ch: '\n' }
            if !active.is_fence()
                && !active.kind.is_bracket()
                && index >= active.start
                && index <= active.at =>
        {
            let popped = state.frames.pop()?;
            let end = popped.at + popped.closer.len();
            let mut written = prev_chars.to_vec();
            written.insert(end.min(written.len()), '\n');
            for outer in &mut state.frames {
                if outer.at >= end {
                    outer.at += 1;
                }
            }
            state.frames.retain(|frame| frame_intact(&written, frame));
            sync_pairing(pairing, state.frames);
            return Some(PairEdit::from_chars(written, end + 1));
        }
        TextDelta::Delete { index }
            if active.is_empty()
                && index >= active.start
                && index < active.start + active.opener.len() =>
        {
            let popped = state.frames.pop()?;
            let mut cleaned = chars.to_vec();
            let at = popped.at - 1;
            let end = (at + popped.closer.len()).min(cleaned.len());
            let removed = end - at;
            cleaned.drain(at..end);
            for outer in &mut state.frames {
                if outer.at >= end {
                    outer.at = outer.at.saturating_sub(1 + removed);
                }
            }
            state.frames.retain(|frame| frame_intact(&cleaned, frame));
            sync_pairing(pairing, state.frames);
            return Some(PairEdit::from_chars(cleaned, caret));
        }
        TextDelta::Insert { index, .. } => {
            shift_frames(&mut state.frames, chars, index, true);
            sync_pairing(pairing, state.frames);
        }
        TextDelta::Delete { index } => {
            shift_frames(&mut state.frames, chars, index, false);
            sync_pairing(pairing, state.frames);
        }
        TextDelta::Other => {
            state.frames = rebase_frames(prev_chars, chars, state.frames);
            sync_pairing(pairing, state.frames);
        }
    }
    None
}

fn try_open_bracket(
    chars: Vec<char>,
    caret: usize,
    bracket: Marker,
    pairing: &mut Option<PairClosing>,
) -> Option<PairEdit> {
    let opener: Vec<char> = bracket.token().chars().collect();
    let rest = &chars[caret..];
    let can_pair = rest.first().is_none_or(|next| {
        next.is_whitespace() || matches!(next, ')' | ']' | '}' | ',' | '.' | ';' | ':')
    });
    if !can_pair || rest.starts_with(&bracket.closer().chars().collect::<Vec<_>>()) {
        return None;
    }
    Some(insert_closer(chars, caret, bracket, opener, pairing))
}

fn insert_closer(
    mut chars: Vec<char>,
    caret: usize,
    kind: Marker,
    opener: Vec<char>,
    pairing: &mut Option<PairClosing>,
) -> PairEdit {
    let closer: Vec<char> = kind.closer().chars().collect();
    for (offset, ch) in closer.iter().enumerate() {
        chars.insert(caret + offset, *ch);
    }
    let mut frames = pairing.take().map_or_else(Vec::new, |p| p.frames);
    for outer in &mut frames {
        if outer.at >= caret {
            outer.at += closer.len();
        }
    }
    frames.push(PairFrame {
        kind,
        start: caret - opener.len(),
        opener,
        closer,
        at: caret,
    });
    sync_pairing(pairing, frames);
    PairEdit::from_chars(chars, caret)
}

/// Tracks non-typing mutations (paste, IME composition) to shift or invalidate pending pairs
/// without ever auto-closing new delimiters.
pub fn track_pair_change(previous: &str, text: &str, pairing: &mut Option<PairClosing>) {
    if previous == text {
        return;
    }
    if let Some(state) = pairing.take() {
        let prev_chars: Vec<char> = previous.chars().collect();
        let next_chars: Vec<char> = text.chars().collect();
        let frames = rebase_frames(&prev_chars, &next_chars, state.frames);
        sync_pairing(pairing, frames);
    }
}

/// Jumps the caret past the innermost active pending closer (typically bound to Tab).
pub fn skip_pair_closer(
    text: &str,
    byte_caret: usize,
    pairing: &mut Option<PairClosing>,
) -> Option<PairEdit> {
    if !text.is_char_boundary(byte_caret) {
        return None;
    }
    let mut state = pairing.clone()?;
    let active = state.frames.pop()?;
    let mut chars: Vec<char> = text.chars().collect();
    if !frame_intact(&chars, &active) {
        *pairing = None;
        return None;
    }
    let char_caret = text[..byte_caret].chars().count();
    if char_caret < active.start || char_caret > active.at {
        return None;
    }
    let end = active.at + active.closer.len();
    let add_space = active.appends_space_on_exit()
        && !active.is_empty()
        && !state.frames.iter().any(|outer| outer.at == end);
    let needs_space = add_space && chars.get(end).is_none_or(|ch| !ch.is_whitespace());
    let next_caret = leave_pair(&mut chars, end, add_space);
    if needs_space {
        for outer in &mut state.frames {
            if outer.at >= end {
                outer.at += 1;
            }
        }
    }
    state.frames.retain(|frame| frame_intact(&chars, frame));
    sync_pairing(pairing, state.frames);
    Some(PairEdit::from_chars(chars, next_caret))
}
