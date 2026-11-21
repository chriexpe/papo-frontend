//! Jumbo emoji (`docs/design.md` §9): a message of one to three emoji,
//! with nothing but spaces between them, draws them large. A grapheme is an
//! emoji by the rule Rune sends it to the colour emoji font with, so what
//! draws large is what draws in colour.
//!
//! Custom emoji are not resolved yet (they wait for the emoji subsystem),
//! so only Unicode emoji count.

use icu_properties::CodePointSetData;
use icu_properties::props::EmojiPresentation;
use papo_sdk_ui::message_content::{Block, Inline, MessageContent, TextStyle};
use unicode_segmentation::UnicodeSegmentation;

use crate::design::JUMBO_MOST;

/// The emoji of a jumbo message, trimmed, or none when it is not one.
pub(super) fn jumbo(content: &MessageContent) -> Option<&str> {
    let [Block::Paragraph(inlines)] = content.blocks() else {
        return None;
    };
    let [
        Inline::Text {
            text,
            style,
            link: None,
            ..
        },
    ] = inlines.as_slice()
    else {
        return None;
    };
    if *style != TextStyle::default() {
        return None;
    }
    let text = text.trim();
    let mut count = 0;
    for grapheme in text.graphemes(true) {
        if grapheme.chars().all(char::is_whitespace) {
            continue;
        }
        if !is_emoji(grapheme) {
            return None;
        }
        count += 1;
    }
    (1..=JUMBO_MOST).contains(&count).then_some(text)
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
    use papo_sdk_ui::message_content::MessageContent;

    use super::jumbo;

    fn jumbo_of(source: &str) -> Option<String> {
        jumbo(&MessageContent::parse(source)).map(str::to_owned)
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
        assert_eq!(
            jumbo_of(":party:"),
            None,
            "custom emoji are not resolved yet"
        );
        assert_eq!(jumbo_of(""), None);
    }
}
