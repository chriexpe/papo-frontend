//! The `:` being typed in a composer and the emoji it can name, and a picker's emoji search,
//! ranked alike (`docs/design.md` §16.2): this server's custom emoji first, then Unicode;
//! within each, names starting with the query before names holding it, in catalog or Unicode
//! order. Accents and case are ignored and a leading or trailing `:` is dropped. The picker,
//! its keys and its looks are the client's.

use std::ops::Range;

use papo_sdk::CustomEmoji;

use crate::unicode_emoji::{UnicodeEmoji, fold, unicode_emoji};

/// What follows a `:` longer than this is no longer taken as a name being typed.
const MAX_QUERY_CHARS: usize = 32;

/// A `:` being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiQuery {
    /// UTF-8 byte range from the `:` to the caret; a picked emoji replaces it.
    pub byte_range: Range<usize>,
    /// What follows the `:`.
    pub text: String,
}

/// One emoji a search or a `:` finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmojiSuggestion {
    /// This server's custom emoji.
    Custom(CustomEmoji),
    /// A Unicode emoji.
    Unicode(&'static UnicodeEmoji),
}

/// The `:` query a collapsed caret at byte offset `caret` of `text` ends, if any: a `:` that
/// starts a word (not after a letter, digit or `_`, so `10:30` and `http:` never ask),
/// followed by at least one character, with no whitespace, `:` or `@` between it and the
/// caret, outside every pill already made (`pills`, UTF-8 byte ranges).
#[must_use]
pub fn emoji_query(text: &str, caret: usize, pills: &[Range<usize>]) -> Option<EmojiQuery> {
    let before = text.get(..caret)?;
    let colon = before.rfind(':')?;
    let typed = &before[colon + 1..];
    if typed.is_empty()
        || typed.chars().count() > MAX_QUERY_CHARS
        || typed
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '@' || ch == ':')
    {
        return None;
    }
    let starts_word = before[..colon]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_');
    if !starts_word
        || pills
            .iter()
            .any(|pill| pill.start < caret && colon < pill.end)
    {
        return None;
    }
    Some(EmojiQuery {
        byte_range: colon..caret,
        text: typed.to_owned(),
    })
}

/// The emoji `query` finds, at most `limit`: `custom` (this server's catalog, in its order)
/// first, then Unicode, each ranked as the module says. An empty query finds nothing.
#[must_use]
pub fn emoji_search(query: &str, custom: &[CustomEmoji], limit: usize) -> Vec<EmojiSuggestion> {
    let folded = fold(query);
    if folded.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, EmojiSuggestion)> = custom
        .iter()
        .filter_map(|emoji| {
            let name = fold(&emoji.name);
            let rank = if name.starts_with(&folded) {
                0
            } else if name.contains(&folded) {
                1
            } else {
                return None;
            };
            Some((rank, EmojiSuggestion::Custom(emoji.clone())))
        })
        .collect();
    // The server's come first whatever their rank: a stable sort keeps catalog order within one.
    found.sort_by_key(|(rank, _)| *rank);
    let mut unicode: Vec<(u8, EmojiSuggestion)> = unicode_emoji()
        .iter()
        .filter_map(|emoji| {
            emoji
                .matches(&folded)
                .map(|rank| (rank, EmojiSuggestion::Unicode(emoji)))
        })
        .collect();
    unicode.sort_by_key(|(rank, _)| *rank);
    found
        .into_iter()
        .chain(unicode)
        .map(|(_, suggestion)| suggestion)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use papo_sdk::Uuid;

    use super::*;

    fn custom(name: &str, id: u128) -> CustomEmoji {
        CustomEmoji {
            id: Uuid::from_u128(id),
            name: name.into(),
            format: "png".into(),
            created_at: "t".into(),
        }
    }

    #[test]
    fn a_colon_starting_a_word_is_a_query_and_a_time_is_not() {
        assert_eq!(
            emoji_query("oi :gat", 7, &[]),
            Some(EmojiQuery {
                byte_range: 3..7,
                text: "gat".into()
            })
        );
        assert_eq!(emoji_query("às 10:30", 8, &[]), None);
        assert_eq!(emoji_query("http:x", 6, &[]), None);
        assert_eq!(emoji_query("oi :", 4, &[]), None, "nothing typed yet");
        assert_eq!(emoji_query("oi :ga to", 9, &[]), None);
        assert_eq!(emoji_query(":gato:", 6, &[]), None, "a closed name");
        let pill = 0..4;
        assert_eq!(
            emoji_query(":gat", 4, std::slice::from_ref(&pill)),
            None,
            "inside a pill"
        );
    }

    #[test]
    fn the_servers_emoji_come_first_then_unicode_prefixes_before_the_rest() {
        let catalog = [
            custom("bigcafe", 1),
            custom("cafezinho", 2),
            custom("dog", 3),
        ];
        let found = emoji_search(":Caf", &catalog, 50);
        let EmojiSuggestion::Custom(first) = &found[0] else {
            panic!("the server's first")
        };
        assert_eq!(
            first.name, "cafezinho",
            "a prefix before a name that holds it"
        );
        assert!(matches!(&found[1], EmojiSuggestion::Custom(emoji) if emoji.name == "bigcafe"));
        assert!(
            found[2..]
                .iter()
                .all(|suggestion| matches!(suggestion, EmojiSuggestion::Unicode(_)))
        );
        assert!(found.iter().any(
            |suggestion| matches!(suggestion, EmojiSuggestion::Unicode(emoji) if emoji.glyph == "☕")
        ));
    }

    #[test]
    fn accents_and_portuguese_words_find_unicode_emoji() {
        let found = emoji_search("coração", &[], 400);
        assert!(found.iter().any(
            |suggestion| matches!(suggestion, EmojiSuggestion::Unicode(emoji) if emoji.glyph == "❤️")
        ));
        assert!(emoji_search("kkk", &[], 5).iter().any(
            |suggestion| matches!(suggestion, EmojiSuggestion::Unicode(emoji) if emoji.glyph == "😂")
        ));
        assert_eq!(emoji_search("::", &[], 5), []);
    }
}
