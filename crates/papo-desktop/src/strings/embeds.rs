//! Link cards' strings (`docs/design.md` §15.8, with drafts for what it lacks). The catalog
//! entries are `embed.*` and Conversations' `settings.embed_size*` and `settings.previews*`.

use super::{I18N, Key, catalog};

/// A card, as assistive technology reads it: "Link preview, {provider}: {title}".
pub(crate) fn embed_card_label(provider: &str, title: &str) -> String {
    template!("embed.card_label", "provider" => provider, "title" => title)
}

/// A card without a title (draft): "Link preview, {provider}".
pub(crate) fn embed_card_label_untitled(provider: &str) -> String {
    template!("embed.card_label_untitled", "provider" => provider)
}

/// A provider video's poster (draft; no length: the server sends none, B-031). Space on the
/// focused message opens the page until the player lands; on the poster it opens the viewer.
pub(crate) fn embed_poster_label(provider: &str, title: &str) -> String {
    template!("embed.poster_label", "provider" => provider, "title" => title)
}

/// A provider video's poster without a title (draft).
pub(crate) fn embed_poster_label_untitled(provider: &str) -> String {
    template!("embed.poster_label_untitled", "provider" => provider)
}

/// The provider line's words when the embed names an author (draft): "{provider} · {author}".
pub(crate) fn embed_provider_author(provider: &str, author: &str) -> String {
    template!("embed.provider_author", "provider" => provider, "author" => author)
}

/// Opens a provider's page in the browser: the poster's play disc and its menu item.
pub(crate) fn embed_open_browser() -> &'static str {
    text!("embed.open_browser")
}

/// Media › Embed size.
pub(crate) fn settings_embed_size() -> &'static str {
    text!("settings.embed_size")
}

/// Embed size's sub-line.
pub(crate) fn settings_embed_size_sub() -> &'static str {
    text!("settings.embed_size.sub")
}

/// Show link previews.
pub(crate) fn settings_previews() -> &'static str {
    text!("settings.previews")
}

/// Show link previews' sub-line.
pub(crate) fn settings_previews_sub() -> &'static str {
    text!("settings.previews.sub")
}
