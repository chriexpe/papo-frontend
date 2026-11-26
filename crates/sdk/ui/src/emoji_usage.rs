//! One per-account count of the emoji used, read two ways: which emoji a message uses (counted
//! by the SDK once it is confirmed, as a reaction is) and the account's emoji ranked by that
//! count, which the picker's Frequently used and the quick reactions share (`docs/design.md`
//! §16, "One ranking"). The count itself is `papo_sdk`'s expression library.

use std::collections::HashSet;

use crate::operations::ReactionEmoji;
use papo_sdk::{EmojiCatalog, MAX_EMOJI_PER_MESSAGE, Uuid};

use crate::message_content::{Block, Inline, MessageContent};
use crate::unicode_emoji::{unicode_emoji_in, unicode_emoji_of};

/// The emoji `content` uses, as the count keeps them (`OutgoingMessage::emoji`), each use
/// counted, at most [`MAX_EMOJI_PER_MESSAGE`]: its custom emoji and the Unicode emoji in its
/// text, not in code.
#[must_use]
pub fn emoji_used(content: &MessageContent) -> Vec<String> {
    let mut used = Vec::new();
    let mut inlines: Vec<&[Inline]> = Vec::new();
    let mut blocks: Vec<&[Block]> = vec![content.blocks()];
    while let Some(level) = blocks.pop() {
        for block in level {
            match block {
                Block::Paragraph(line) | Block::Heading { inlines: line, .. } => inlines.push(line),
                Block::BlockQuote(inner) => blocks.push(inner),
                Block::List { items, .. } => {
                    blocks.extend(items.iter().map(|item| item.blocks.as_slice()));
                }
                Block::Table { header, rows, .. } => {
                    inlines.extend(header.iter().map(Vec::as_slice));
                    for row in rows {
                        inlines.extend(row.iter().map(Vec::as_slice));
                    }
                }
                Block::CodeBlock { .. } | Block::ThematicBreak | Block::BlankLine => {}
            }
        }
    }
    // Blocks and lines are walked from the end; the count doesn't depend on the order.
    while let Some(line) = inlines.pop() {
        for inline in line {
            match inline {
                Inline::Text { text, .. } => used.extend(
                    unicode_emoji_in(text)
                        .into_iter()
                        .map(|emoji| emoji.glyph.to_owned()),
                ),
                Inline::CustomEmoji { id, .. } => used.push(id.to_string()),
                Inline::Spoiler(inner) => inlines.push(inner),
                _ => {}
            }
        }
    }
    used.truncate(MAX_EMOJI_PER_MESSAGE);
    used
}

/// The account's `count` most used emoji, most used first (`usage` is the expression library's
/// `emoji_usage`): custom emoji only while `catalog` still has them, Unicode ones only when
/// they are emoji, each as the account used it (the server tells "❤" from "❤️"), and one
/// heart for both.
#[must_use]
pub fn frequent_emoji(
    usage: &[(String, u64)],
    catalog: Option<&EmojiCatalog>,
    count: usize,
) -> Vec<ReactionEmoji> {
    let mut frequent = Vec::with_capacity(count);
    let mut seen = HashSet::new();
    for (key, _) in usage {
        if frequent.len() == count {
            break;
        }
        let Some((emoji, drawn)) = emoji_of_key(key, catalog) else {
            continue;
        };
        if seen.insert(drawn) {
            frequent.push(emoji);
        }
    }
    frequent
}

/// The emoji count key `key` names, when it still is one, and what it draws as (a Unicode
/// emoji's table glyph), which tells the same emoji typed two ways apart from two emoji.
fn emoji_of_key(key: &str, catalog: Option<&EmojiCatalog>) -> Option<(ReactionEmoji, String)> {
    if let Ok(id) = Uuid::parse_str(key) {
        return catalog?
            .get(id)
            .map(|_| (ReactionEmoji::Custom(id), key.to_owned()));
    }
    unicode_emoji_of(key).map(|emoji| {
        (
            ReactionEmoji::Unicode(key.to_owned()),
            emoji.glyph.to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    use papo_sdk::CustomEmoji;

    use super::*;
    use crate::message_content::CustomEmojiRef;

    const GATO: Uuid = Uuid::from_u128(7);

    fn catalog() -> EmojiCatalog {
        EmojiCatalog {
            revision: 1,
            emojis: vec![CustomEmoji {
                id: GATO,
                name: "gato".into(),
                format: "png".into(),
                created_at: "t".into(),
            }],
        }
    }

    #[test]
    fn a_message_uses_its_custom_and_unicode_emoji_but_not_code() {
        let content = MessageContent::parse_with_custom_emojis(
            "oi :gato: 😂😂 `🔥` > ❤️",
            &[CustomEmojiRef {
                id: GATO,
                name: "gato".into(),
            }],
        );
        let mut used = emoji_used(&content);
        used.sort();
        let mut expected = vec![GATO.to_string(), "😂".into(), "😂".into(), "❤️".into()];
        expected.sort();
        assert_eq!(used, expected);
    }

    #[test]
    fn frequent_emoji_keep_what_still_exists_and_one_heart() {
        let gone = Uuid::from_u128(99).to_string();
        let usage = vec![
            (gone, 9),
            ("❤".to_owned(), 8),
            (GATO.to_string(), 7),
            ("❤️".to_owned(), 6),
            ("not an emoji".to_owned(), 5),
            ("😂".to_owned(), 4),
        ];
        let frequent = frequent_emoji(&usage, Some(&catalog()), 8);
        assert_eq!(
            frequent,
            [
                ReactionEmoji::Unicode("❤".into()),
                ReactionEmoji::Custom(GATO),
                ReactionEmoji::Unicode("😂".into()),
            ]
        );
        assert_eq!(
            frequent_emoji(&usage, None, 8).len(),
            2,
            "no catalog, no custom"
        );
    }
}
