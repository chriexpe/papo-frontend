//! The Unicode emoji a client offers: each one's glyph, category, `:code:`, English and
//! Portuguese names, and the words it is found by (`data.rs`, generated from Unicode's and
//! CLDR's data by `scripts/gen-unicode-emoji.py`). Also which emoji a text holds, by the same
//! table. Drawing them is the system's colour emoji font's; ranking a search is
//! `emoji_suggestions`'.

use std::collections::HashMap;
use std::sync::OnceLock;

// Generated: kept exactly as the generator writes it.
#[rustfmt::skip]
mod data;

/// A picker's Unicode sections, in the order it shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmojiCategory {
    /// Smileys and people (Unicode's "Smileys & Emotion" and "People & Body").
    SmileysAndPeople,
    /// Animals and nature.
    AnimalsAndNature,
    /// Food and drink.
    FoodAndDrink,
    /// Activities.
    Activities,
    /// Travel and places.
    TravelAndPlaces,
    /// Objects.
    Objects,
    /// Symbols.
    Symbols,
    /// Flags.
    Flags,
}

impl EmojiCategory {
    /// Every category, in the picker's order.
    pub const ALL: [EmojiCategory; 8] = [
        EmojiCategory::SmileysAndPeople,
        EmojiCategory::AnimalsAndNature,
        EmojiCategory::FoodAndDrink,
        EmojiCategory::Activities,
        EmojiCategory::TravelAndPlaces,
        EmojiCategory::Objects,
        EmojiCategory::Symbols,
        EmojiCategory::Flags,
    ];
}

/// Which language an emoji's name is wanted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiLanguage {
    /// CLDR's English name, as Unicode writes it.
    English,
    /// CLDR's Portuguese name.
    Portuguese,
}

/// One Unicode emoji.
#[derive(Debug, PartialEq, Eq)]
pub struct UnicodeEmoji {
    /// The emoji, fully qualified (with U+FE0F where Unicode asks for it).
    pub glyph: &'static str,
    /// Its picker section.
    pub category: EmojiCategory,
    /// Its code without colons: the English name in lower case, words joined by `_`
    /// (`face_with_tears_of_joy`, `flag_brazil`).
    pub code: &'static str,
    name_en: &'static str,
    name_pt: &'static str,
    /// Its names and CLDR's words for it, in both languages, folded and joined by `|`.
    search: &'static str,
}

impl UnicodeEmoji {
    /// Its name in `language`: what assistive technology says for it.
    #[must_use]
    pub fn name(&self, language: EmojiLanguage) -> &'static str {
        match language {
            EmojiLanguage::English => self.name_en,
            EmojiLanguage::Portuguese => self.name_pt,
        }
    }

    /// Where `folded` (a [`fold`]ed query) is found among its words: 0 when a word starts with
    /// it, 1 when a word holds it, none when nothing does.
    pub(crate) fn matches(&self, folded: &str) -> Option<u8> {
        let mut found = None;
        for word in self.search.split('|') {
            if word.starts_with(folded) || self.code.starts_with(folded) {
                return Some(0);
            }
            if word.contains(folded) {
                found = Some(1);
            }
        }
        found
    }
}

/// Every Unicode emoji offered, in Unicode's order.
#[must_use]
pub fn unicode_emoji() -> &'static [UnicodeEmoji] {
    &data::EMOJI
}

/// The emoji drawn as `glyph`, with or without its U+FE0F presentation selectors.
#[must_use]
pub fn unicode_emoji_of(glyph: &str) -> Option<&'static UnicodeEmoji> {
    by_glyph()
        .get(bare(glyph).as_str())
        .map(|at| &data::EMOJI[*at])
}

/// The Unicode emoji `text` holds, in order, repeats included: the longest one at each place,
/// so a family or a flag is one emoji. A character that is text unless U+FE0F follows it
/// (`©`, `™`, `❤`) counts only with it.
#[must_use]
pub fn unicode_emoji_in(text: &str) -> Vec<&'static UnicodeEmoji> {
    let index = by_glyph();
    let longest = longest_bare();
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let mut matched = None;
        // Try the longest candidate first: char boundaries only, up to the longest glyph.
        let ends: Vec<usize> = rest
            .char_indices()
            .map(|(offset, ch)| offset + ch.len_utf8())
            .take(longest)
            .collect();
        for end in ends.into_iter().rev() {
            if let Some(index) = index.get(bare(&rest[..end]).as_str()) {
                // A trailing U+FE0F belongs to the emoji it follows.
                let end = if rest[end..].starts_with('\u{fe0f}') {
                    end + '\u{fe0f}'.len_utf8()
                } else {
                    end
                };
                let as_text = data::EMOJI[*index].glyph.contains('\u{fe0f}')
                    && !rest[..end].contains('\u{fe0f}');
                if !as_text {
                    matched = Some((*index, end));
                }
                break;
            }
        }
        match matched {
            Some((index, end)) => {
                found.push(&data::EMOJI[index]);
                at += end;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    found
}

/// `text` as emoji words are kept: lower case, the accents Portuguese and Spanish use dropped,
/// `_` and `-` as spaces, and a leading or trailing `:` gone (`:Coração_Partido` → `coracao
/// partido`). Matches `scripts/gen-unicode-emoji.py`'s `fold`.
#[must_use]
pub fn fold(text: &str) -> String {
    let text = text.trim().trim_start_matches(':').trim_end_matches(':');
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| match ch {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            '_' | '-' => ' ',
            other => other,
        })
        .collect()
}

/// `glyph` without its U+FE0F presentation selectors: one emoji, however it was typed.
fn bare(glyph: &str) -> String {
    glyph.chars().filter(|ch| *ch != '\u{fe0f}').collect()
}

/// Each emoji's place in the table by its bare glyph.
fn by_glyph() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        data::EMOJI
            .iter()
            .enumerate()
            .map(|(at, emoji)| (bare(emoji.glyph), at))
            .collect()
    })
}

/// The most chars a bare glyph has.
fn longest_bare() -> usize {
    static LONGEST: OnceLock<usize> = OnceLock::new();
    *LONGEST.get_or_init(|| {
        data::EMOJI
            .iter()
            .map(|emoji| bare(emoji.glyph).chars().count())
            .max()
            .unwrap_or(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_emoji_and_every_code_is_unique() {
        for category in EmojiCategory::ALL {
            assert!(
                unicode_emoji()
                    .iter()
                    .any(|emoji| emoji.category == category)
            );
        }
        let mut codes = std::collections::HashSet::new();
        for emoji in unicode_emoji() {
            assert!(codes.insert(emoji.code), "{} twice", emoji.code);
        }
    }

    #[test]
    fn an_emoji_has_its_names_and_code() {
        let joy = unicode_emoji_of("😂").unwrap();
        assert_eq!(joy.code, "face_with_tears_of_joy");
        assert_eq!(joy.name(EmojiLanguage::English), "face with tears of joy");
        assert_eq!(joy.name(EmojiLanguage::Portuguese), "rosto chorando de rir");
        assert_eq!(
            unicode_emoji_of("❤").map(|heart| heart.glyph),
            Some("❤️"),
            "with or without its selector"
        );
    }

    #[test]
    fn a_query_folds_like_the_table() {
        assert_eq!(fold(":Coração_Partido:"), "coracao partido");
        let broken = unicode_emoji_of("💔").unwrap();
        assert_eq!(broken.matches(&fold("coração")), Some(0));
        assert_eq!(broken.matches(&fold("partido")), Some(0));
        assert_eq!(broken.matches(&fold("zzz")), None);
    }

    #[test]
    fn the_emoji_a_text_holds_are_found_whole() {
        let found: Vec<&str> = unicode_emoji_in("oi 😂 👨‍👩‍👧‍👦 e 🇧🇷! ❤️ ok © ™")
            .into_iter()
            .map(|emoji| emoji.glyph)
            .collect();
        assert_eq!(found, ["😂", "👨‍👩‍👧‍👦", "🇧🇷", "❤️"], "© and ™ are text");
        assert_eq!(
            unicode_emoji_in("10:30 :gato: abc"),
            [] as [&UnicodeEmoji; 0]
        );
    }
}
