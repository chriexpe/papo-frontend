//! Link cards' and provider players' strings (`docs/design.md` §15.8, with drafts for what it
//! lacks). The catalog entries are `embed.*`, Conversations' `settings.embed_size*` and
//! `settings.previews*`, and General's `settings.players*`.

use super::{I18N, Key, PluralKey, catalog};

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

/// The poster's play disc, and the ask's Play.
pub(crate) fn embed_play() -> &'static str {
    text!("embed.play")
}

/// The ask on the poster: what loading the player tells the provider.
pub(crate) fn embed_consent(provider: &str) -> String {
    template!("embed.consent", "provider" => provider)
}

/// The ask's Always allow.
pub(crate) fn embed_always(provider: &str) -> String {
    template!("embed.always", "provider" => provider)
}

/// The ask's Cancel.
pub(crate) fn embed_cancel() -> &'static str {
    text!("embed.cancel")
}

/// The ask, as a group assistive technology names (draft, §15.6).
pub(crate) fn embed_ask_label(provider: &str) -> String {
    template!("embed.ask_label", "provider" => provider)
}

/// A live player's Close player, its button and menu item.
pub(crate) fn embed_close_player() -> &'static str {
    text!("embed.close_player")
}

/// A live player's region, spoken (draft, §15.6).
pub(crate) fn embed_player_label(provider: &str, title: Option<&str>) -> String {
    if let Some(title) = title {
        template!("embed.player_label", "provider" => provider, "title" => title)
    } else {
        template!("embed.player_label_untitled", "provider" => provider)
    }
}

/// The note on a released player's poster.
pub(crate) fn embed_stopped() -> &'static str {
    text!("embed.stopped")
}

/// A released player's poster, spoken (draft, §15.6).
pub(crate) fn embed_stopped_label() -> &'static str {
    text!("embed.stopped_label")
}

/// A player that couldn't load.
pub(crate) fn embed_failed() -> &'static str {
    text!("embed.failed")
}

/// A failed player's Retry.
pub(crate) fn embed_retry() -> &'static str {
    text!("embed.retry")
}

/// A provider video's Open viewer menu item (draft, §15.6).
pub(crate) fn embed_open_viewer() -> &'static str {
    text!("embed.open_viewer")
}

/// The provider viewer, as the dialog assistive technology names (draft).
pub(crate) fn embed_viewer_label(provider: &str, title: &str) -> String {
    template!("embed.viewer_label", "provider" => provider, "title" => title)
}

/// General › Embedded players.
pub(crate) fn settings_players() -> &'static str {
    text!("settings.players")
}

/// Embedded players' summary on General (draft): how many sites are allowed.
pub(crate) fn settings_players_sub(count: usize) -> String {
    plural!(
        "settings.players_sub",
        u64::try_from(count).unwrap_or(u64::MAX)
    )
}

/// Play videos and music from other sites.
pub(crate) fn settings_players_on() -> &'static str {
    text!("settings.players_on")
}

/// Its sub-line.
pub(crate) fn settings_players_on_sub() -> &'static str {
    text!("settings.players_on.sub")
}

/// The sites' group title (draft).
pub(crate) fn settings_players_sites() -> &'static str {
    text!("settings.players_sites")
}

/// No site allowed or with data (draft).
pub(crate) fn settings_players_empty() -> &'static str {
    text!("settings.players_empty")
}

/// A site's Clear data (draft).
pub(crate) fn settings_players_clear() -> &'static str {
    text!("settings.players_clear")
}

/// A site's Clear data, spoken (draft).
pub(crate) fn settings_players_clear_spoken(site: &str) -> String {
    template!("settings.players_clear_spoken", "site" => site)
}

/// A site's Remove, spoken (draft).
pub(crate) fn settings_players_remove_spoken(site: &str) -> String {
    template!("settings.players_remove_spoken", "site" => site)
}

/// The note under the sites: Remove.
pub(crate) fn settings_players_list_note() -> &'static str {
    text!("settings.players_list.note")
}

/// The note under the sites: Clear data (draft).
pub(crate) fn settings_players_clear_note() -> &'static str {
    text!("settings.players_clear.note")
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
