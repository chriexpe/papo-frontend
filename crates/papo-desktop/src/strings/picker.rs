//! The strings of the emoji picker (`docs/design.md` §16.6) and More emoji (§10). The catalog
//! entries are `picker.*`, `emoji.category.*` and `reaction.more_emoji`.

use papo_sdk_ui::{EmojiCategory, EmojiLanguage};

use super::{I18N, Key, catalog};

/// The picker as a dialog, and the pill's 😊 button: "Emoji".
pub(crate) fn picker_emoji() -> &'static str {
    text!("picker.emoji")
}

/// The `:` suggestions over the field, as a list.
pub(crate) fn emoji_suggestions() -> &'static str {
    text!("picker.emoji_suggestions")
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

/// The picker as a dialog on the GIF tab, and the pill's GIF button: "GIFs".
pub(crate) fn picker_gifs() -> &'static str {
    text!("picker.gifs")
}

/// The GIF tab's search field.
pub(crate) fn picker_search_gifs() -> &'static str {
    text!("picker.search_gifs")
}

/// The GIF chips: Favorites, Recent, Trending.
pub(crate) fn picker_favorites() -> &'static str {
    text!("picker.favorites")
}

pub(crate) fn picker_recent() -> &'static str {
    text!("picker.recent")
}

pub(crate) fn picker_trending() -> &'static str {
    text!("picker.trending")
}

/// Favorites with none yet.
pub(crate) fn picker_fav_empty() -> &'static str {
    text!("picker.fav_empty")
}

/// A search GIPHY has nothing for: "No GIFs for “{q}”".
pub(crate) fn picker_no_gifs(query: &str) -> String {
    template!("picker.no_gifs", "q" => query)
}

/// A build without a GIPHY key, in the search field's place.
pub(crate) fn picker_no_key() -> &'static str {
    text!("picker.no_key")
}

/// GIPHY unreachable, and its button.
pub(crate) fn picker_failed() -> &'static str {
    text!("picker.failed")
}

pub(crate) fn picker_try_again() -> &'static str {
    text!("picker.try_again")
}

/// Under the tiles while the next page comes.
pub(crate) fn picker_loading_more() -> &'static str {
    text!("picker.loading_more")
}

/// GIPHY's attribution, at the search field's right end (GIPHY's name: not translated).
pub(crate) fn picker_powered() -> &'static str {
    text!("picker.powered")
}

/// A GIF tile as spoken: "GIF, {title}", or "GIF" without one.
pub(crate) fn picker_gif_tile(title: &str) -> String {
    if title.trim().is_empty() {
        return text!("picker.gif_untitled").to_owned();
    }
    template!("picker.gif_tile", "title" => title)
}

/// Ctrl+G, the GIF tab's key.
pub(crate) fn shortcut_gif() -> &'static str {
    text!("shortcut.gif")
}

/// The star and the menus: "Add to favorites" / "Remove from favorites".
pub(crate) fn fav_add() -> &'static str {
    text!("fav.add")
}

pub(crate) fn fav_remove() -> &'static str {
    text!("fav.remove")
}

/// Said once, politely, when a GIF becomes a favorite.
pub(crate) fn fav_added() -> &'static str {
    text!("fav.added")
}

/// A favorite tile, after its title: "favorite".
pub(crate) fn fav_is() -> &'static str {
    text!("fav.is")
}

/// A favorite the device did not keep.
pub(crate) fn fav_failed() -> &'static str {
    text!("fav.failed")
}

/// S, on a focused GIF message and beside the menus' items.
pub(crate) fn shortcut_favorite() -> &'static str {
    text!("shortcut.favorite")
}
