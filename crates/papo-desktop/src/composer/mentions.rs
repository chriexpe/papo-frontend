//! Mentions in the field: each one a Rune pill carrying the person's id, so
//! the field keeps it whole and in place across edits, undo and redo. On
//! send and save the pills become the SDK's bindings and the text its wire
//! form (`encode_composer_mentions`); a restored draft's wire form becomes
//! pills again (`decode_composer_mentions`, run where the names are).
//!
//! Also the @ query at the caret and the people it finds, from the
//! account's roster (`docs/design.md` §3, "Composer surfaces"). Pure.

use std::ops::Range;

use papo_sdk::{ResourceSnapshot, Uuid};
use papo_sdk_ui::message_content::{
    ComposerMentionBinding, DecodedComposerMentions, encode_composer_mentions, utf8_range_to_utf16,
};
use rune::TextEdit;

/// The most people the picker lists; a longer query narrows it.
const MAX_CANDIDATES: usize = 50;

/// The longest name a query matches, in characters: past it, the `@` is
/// just text.
const MAX_QUERY_CHARS: usize = 32;

/// The text that mentions everyone.
const EVERYONE: &str = "@everyone";

/// The SDK's bindings for the field's pills, in text order.
pub(crate) fn bindings(edit: &TextEdit) -> Vec<ComposerMentionBinding> {
    edit.pills()
        .filter_map(|(range, id)| {
            let id = Uuid::parse_str(id).ok()?;
            let utf16_range = utf8_range_to_utf16(&edit.text, &range)?;
            let text = edit.text.get(range.clone())?.to_owned();
            Some(ComposerMentionBinding {
                id,
                byte_range: range,
                utf16_range,
                text,
            })
        })
        .collect()
}

/// The field's text as sent and stored: each pill its mention token.
pub(crate) fn wire(edit: &TextEdit) -> String {
    let bindings = bindings(edit);
    if bindings.is_empty() {
        return edit.text.clone();
    }
    encode_composer_mentions(&edit.text, &bindings)
}

/// Puts `decoded` in the field with its mentions as pills, the caret at
/// the end and no undo history: a restored draft is where editing starts.
pub(crate) fn restore(edit: &mut TextEdit, decoded: DecodedComposerMentions) {
    edit.replace_text(decoded.text);
    for binding in decoded.bindings {
        // The label is the text already there, so no offset moves.
        edit.replace_with_pill(binding.byte_range, binding.id.to_string(), &binding.text);
    }
    edit.clear_history();
    edit.set_caret(edit.text.len());
}

/// Puts `decoded` (a message the SDK did not queue) before what the field
/// holds, on a line of its own, with its mentions as pills; the field's
/// own pills move along.
pub(crate) fn prepend(edit: &mut TextEdit, decoded: DecodedComposerMentions) {
    if edit.text.is_empty() {
        restore(edit, decoded);
        return;
    }
    edit.set_caret(0);
    edit.insert(&format!("{}\n", decoded.text));
    for binding in decoded.bindings {
        edit.replace_with_pill(binding.byte_range, binding.id.to_string(), &binding.text);
    }
    edit.set_caret(edit.text.len());
}

/// An `@` being typed: the range from the `@` to the caret, and what
/// follows the `@`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

/// The @ query the caret ends, if any: an `@` at the start of a word, with
/// no space between it and the caret, outside any mention already made.
pub(crate) fn query(edit: &TextEdit) -> Option<Query> {
    let (start, caret) = edit.range();
    if start != caret {
        return None;
    }
    let before = edit.text.get(..caret)?;
    let at = before.rfind('@')?;
    let typed = &before[at + 1..];
    if typed.chars().any(char::is_whitespace) || typed.chars().count() > MAX_QUERY_CHARS {
        return None;
    }
    // "ana@example.org" is an address, not a mention.
    if before[..at]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    if edit.pills().any(|(range, _)| range.contains(&at)) {
        return None;
    }
    Some(Query {
        range: at..caret,
        text: typed.to_owned(),
    })
}

/// Who a picked row mentions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Everyone,
    User(Uuid),
}

/// One row of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) target: Target,
    /// The nickname, else the username: what the mention shows.
    pub(crate) name: String,
    pub(crate) username: Option<String>,
}

/// The people `query` finds in `snapshot`'s roster: names starting with it
/// first, then names containing it, in roster order, ignoring case.
/// @everyone leads when `everyone` allows it and it matches.
pub(crate) fn candidates(
    snapshot: &ResourceSnapshot,
    query: &str,
    everyone: bool,
) -> Vec<Candidate> {
    let query = query.to_lowercase();
    let mut starting = Vec::new();
    let mut containing = Vec::new();
    for id in snapshot.members.iter() {
        let Some(user) = snapshot.users.get(id) else {
            continue;
        };
        let Some(name) = user.nickname.as_ref().or(user.username.as_ref()) else {
            continue;
        };
        let names = [Some(name), user.username.as_ref()];
        let lowered: Vec<String> = names.iter().flatten().map(|n| n.to_lowercase()).collect();
        let candidate = || Candidate {
            target: Target::User(id),
            name: name.clone(),
            username: user.username.clone(),
        };
        if lowered.iter().any(|n| n.starts_with(&query)) {
            starting.push(candidate());
        } else if lowered.iter().any(|n| n.contains(&query)) {
            containing.push(candidate());
        }
        if starting.len() >= MAX_CANDIDATES {
            break;
        }
    }
    let lead = (everyone && EVERYONE[1..].starts_with(&query)).then(|| Candidate {
        target: Target::Everyone,
        name: EVERYONE[1..].to_owned(),
        username: None,
    });
    lead.into_iter()
        .chain(starting)
        .chain(containing)
        .take(MAX_CANDIDATES)
        .collect()
}

/// Replaces the query with the mention of `candidate` and a space after it.
pub(crate) fn pick(edit: &mut TextEdit, query: &Query, candidate: &Candidate) {
    match candidate.target {
        Target::User(id) => {
            edit.replace_with_pill(
                query.range.clone(),
                id.to_string(),
                &format!("@{}", candidate.name),
            );
            edit.insert(" ");
        }
        Target::Everyone => {
            edit.set_selection(query.range.start, query.range.end);
            edit.insert(&format!("{EVERYONE} "));
        }
    }
}

#[cfg(test)]
mod tests;
