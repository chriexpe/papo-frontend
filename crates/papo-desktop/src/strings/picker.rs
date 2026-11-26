//! The strings of the emoji picker (`docs/design.md` §16.6) and More emoji (§10). The catalog
//! entries are `picker.*`, `emoji.category.*` and `reaction.more_emoji`.

use papo_sdk_ui::{EmojiCategory, EmojiLanguage};

use super::{I18N, Key, catalog};

/// The picker as a dialog, and the pill's 😊 button: "Emoji".
pub(crate) fn picker_emoji() -> &'static str {
    text!("picker.emoji")
}

/// The Emoji tab's search field.
pub(crate) fn picker_search_emoji() -> &'static str {
    text!("picker.search_emoji")
}

/// Its ✕.
pub(crate) fn picker_clear() -> &'static str {
    text!("picker.clear")
}

/// The Frequently used section.
pub(crate) fn picker_frequent() -> &'static str {
    text!("picker.frequent")
}

/// Where a Unicode emoji is from, and a search's Unicode section.
pub(crate) fn picker_unicode() -> &'static str {
    text!("picker.unicode")
}

/// The preview bar with nothing under the pointer.
pub(crate) fn picker_shift_hint() -> &'static str {
    text!("picker.shift_hint")
}

/// A search that finds nothing: "No emoji match “{q}”".
pub(crate) fn picker_no_emoji(query: &str) -> String {
    template!("picker.no_emoji", "q" => query)
}

/// The language Unicode emoji are named in: the app's, where CLDR has it.
pub(crate) fn emoji_language() -> EmojiLanguage {
    if I18N.locales()[super::locale_index()].id().starts_with("pt") {
        EmojiLanguage::Portuguese
    } else {
        EmojiLanguage::English
    }
}

/// Ctrl+E, the Emoji tab's key.
pub(crate) fn shortcut_emoji() -> &'static str {
    text!("shortcut.emoji")
}

/// A Unicode category's name.
pub(crate) fn emoji_category(category: EmojiCategory) -> &'static str {
    match category {
        EmojiCategory::SmileysAndPeople => text!("emoji.category.smileys"),
        EmojiCategory::AnimalsAndNature => text!("emoji.category.nature"),
        EmojiCategory::FoodAndDrink => text!("emoji.category.food"),
        EmojiCategory::Activities => text!("emoji.category.activities"),
        EmojiCategory::TravelAndPlaces => text!("emoji.category.travel"),
        EmojiCategory::Objects => text!("emoji.category.objects"),
        EmojiCategory::Symbols => text!("emoji.category.symbols"),
        EmojiCategory::Flags => text!("emoji.category.flags"),
    }
}

/// More emoji, at the quick-reaction strip's left end.
pub(crate) fn more_emoji() -> &'static str {
    text!("reaction.more_emoji")
}
