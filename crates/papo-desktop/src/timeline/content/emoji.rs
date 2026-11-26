//! Emoji in message content (`docs/design.md` §9): jumbo, a message of one to three emoji
//! (custom or Unicode) with nothing but spaces between them, drawn large; and which custom
//! emoji a message names, for their images. A grapheme is a Unicode emoji by the rule Rune
//! sends it to the colour emoji font with, so what draws large is what draws in colour.

use icu_properties::CodePointSetData;
use icu_properties::props::EmojiPresentation;
use papo_sdk::Uuid;
use papo_sdk_ui::message_content::{Block, Inline, MessageContent, TextStyle};
use unicode_segmentation::UnicodeSegmentation;

use crate::design::JUMBO_MOST;

/// One piece of a jumbo message, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum JumboPart<'a> {
    /// Unicode emoji and the spaces between them.
    Text(&'a str),
    /// A custom emoji: its id and its `:name:`.
    Custom { id: Uuid, raw: &'a str },
}

/// The pieces of a jumbo message, outer spaces trimmed, or none when it is not one.
pub(super) fn jumbo(content: &MessageContent) -> Option<Vec<JumboPart<'_>>> {
    let [Block::Paragraph(inlines)] = content.blocks() else {
        return None;
    };
    let mut parts = Vec::new();
    let mut count = 0;
    for inline in inlines {
        match inline {
            Inline::Text {
                text,
                style,
                link: None,
                ..
            } if *style == TextStyle::default() => {
                for grapheme in text.graphemes(true) {
                    if grapheme.chars().all(char::is_whitespace) {
                        continue;
                    }
                    if !is_emoji(grapheme) {
                        return None;
                    }
                    count += 1;
                }
                parts.push(JumboPart::Text(text));
            }
            Inline::CustomEmoji { id, raw } => {
                count += 1;
                parts.push(JumboPart::Custom { id: *id, raw });
            }
            _ => return None,
        }
    }
    if !(1..=JUMBO_MOST).contains(&count) {
        return None;
    }
    trim_ends(&mut parts);
    Some(parts)
}

/// Drops the spaces before the first emoji and after the last.
fn trim_ends(parts: &mut Vec<JumboPart<'_>>) {
    if let Some(JumboPart::Text(text)) = parts.first_mut() {
        *text = text.trim_start();
    }
    if let Some(JumboPart::Text(text)) = parts.last_mut() {
        *text = text.trim_end();
    }
    parts.retain(|part| !matches!(part, JumboPart::Text("")));
}

/// The custom emoji `content` names, in order, repeats included.
pub(crate) fn custom_emoji_ids(content: &MessageContent) -> Vec<Uuid> {
    let mut ids = Vec::new();
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
    while let Some(line) = inlines.pop() {
        for inline in line {
            match inline {
                Inline::CustomEmoji { id, .. } => ids.push(*id),
                Inline::Spoiler(inner) => inlines.push(inner),
                _ => {}
            }
        }
    }
    ids
}

/// Whether grapheme `grapheme` draws as an emoji: `U+FE0F`, a character
/// shown as emoji by default (✅ ⚡ 🔥, not ☺ ✓ ©), a skin tone, a keycap, a
/// flag or a tag sequence, unless `U+FE0E` asks for text.
fn is_emoji(grapheme: &str) -> bool {
    if grapheme.contains('\u{FE0E}') {
        return false;
    }
    let sequence = grapheme.chars().any(|c| {
        matches!(
            u32::from(c),
            0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0x1F1E6..=0x1F1FF | 0xE0020..=0xE007F
        )
    });
    sequence
        || grapheme
            .chars()
            .next()
            .is_some_and(|c| CodePointSetData::new::<EmojiPresentation>().contains(c))
}

#[cfg(test)]
mod tests {
    use papo_sdk::Uuid;
    use papo_sdk_ui::message_content::{CustomEmojiRef, MessageContent};

    use super::{JumboPart, custom_emoji_ids, jumbo};

    fn gato() -> CustomEmojiRef {
        CustomEmojiRef {
            id: Uuid::from_u128(7),
            name: "gato".into(),
        }
    }

    fn parse(source: &str) -> MessageContent {
        MessageContent::parse_with_custom_emojis(source, &[gato()])
    }

    /// The jumbo message's Unicode text, its custom emoji as `<name>`.
    fn jumbo_of(source: &str) -> Option<String> {
        let content = parse(source);
        jumbo(&content).map(|parts| {
            parts
                .iter()
                .map(|part| match part {
                    JumboPart::Text(text) => (*text).to_owned(),
                    JumboPart::Custom { raw, .. } => format!("<{raw}>"),
                })
                .collect()
        })
    }

    #[test]
    fn one_to_three_emoji_with_spaces_between_are_jumbo() {
        assert_eq!(jumbo_of("🔥").as_deref(), Some("🔥"));
        assert_eq!(jumbo_of(" 🎉 👍🏽  🇧🇷 ").as_deref(), Some("🎉 👍🏽  🇧🇷"));
        assert_eq!(
            jumbo_of("👨‍👩‍👧‍👦").as_deref(),
            Some("👨‍👩‍👧‍👦"),
            "a family is one emoji"
        );
        assert_eq!(jumbo_of("❤️").as_deref(), Some("❤️"));
    }

    #[test]
    fn four_emoji_text_or_formatting_are_not_jumbo() {
        assert_eq!(jumbo_of("🔥🔥🔥🔥"), None);
        assert_eq!(jumbo_of("🔥 ok"), None);
        assert_eq!(jumbo_of("**🔥**"), None);
        assert_eq!(jumbo_of("✓"), None, "a check mark is text by default");
        assert_eq!(jumbo_of(":party:"), None, "a name the server lacks is text");
        assert_eq!(jumbo_of(""), None);
    }

    #[test]
    fn custom_emoji_count_toward_jumbo() {
        assert_eq!(jumbo_of(" :gato: ").as_deref(), Some("<:gato:>"));
        assert_eq!(
            jumbo_of(":gato: 🔥 :gato:").as_deref(),
            Some("<:gato:> 🔥 <:gato:>")
        );
        assert_eq!(
            jumbo_of(":gato::gato::gato::gato:"),
            None,
            "four are text size"
        );
        assert_eq!(jumbo_of(":gato: ok"), None);
    }

    #[test]
    fn every_custom_emoji_named_is_found_in_quotes_lists_and_spoilers() {
        let content = parse("> :gato:\n\n- ||:gato:||\n\n`:gato:`");
        assert_eq!(custom_emoji_ids(&content), [gato().id, gato().id]);
    }
}
