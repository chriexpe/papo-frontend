//! The search panel's strings (`docs/design.md` §10, Search). The catalog
//! entries are `search.*` and `shortcut.search`.

use super::{I18N, Key, PluralKey, catalog};

/// 🔍's name, and the panel's.
pub(crate) fn search_title() -> &'static str {
    text!("search.title")
}

/// The empty field on a server: "Search Cyber Asilo".
pub(crate) fn search_placeholder(server: &str) -> String {
    template!("search.placeholder", "server" => server)
}

/// The empty field in a DM: "Search @Lia".
pub(crate) fn search_placeholder_dm(name: &str) -> String {
    template!("search.placeholder_dm", "name" => name)
}

/// The filter list's heading.
pub(crate) fn search_filters() -> &'static str {
    text!("search.filters")
}

/// The author filter's keyword in this locale, colon included.
pub(crate) fn search_keyword_from() -> &'static str {
    text!("search.keyword_from")
}

/// The channel filter's keyword in this locale, colon included.
pub(crate) fn search_keyword_in() -> &'static str {
    text!("search.keyword_in")
}

/// The link-or-file filter's keyword in this locale, colon included.
pub(crate) fn search_keyword_has() -> &'static str {
    text!("search.keyword_has")
}

/// The mention filter's keyword in this locale, colon included.
pub(crate) fn search_keyword_mentions() -> &'static str {
    text!("search.keyword_mentions")
}

/// What `from:` and `mentions:` take.
pub(crate) fn search_hint_person() -> &'static str {
    text!("search.hint_person")
}

/// What `in:` takes.
pub(crate) fn search_hint_channel() -> &'static str {
    text!("search.hint_channel")
}

/// What `has:` takes.
pub(crate) fn search_hint_has() -> &'static str {
    text!("search.hint_has")
}

/// `has:link`'s value, as written and listed.
pub(crate) fn search_has_link() -> &'static str {
    text!("search.has_link")
}

/// `has:file`'s value, as written and listed in this locale.
pub(crate) fn search_has_file() -> &'static str {
    text!("search.has_file")
}

/// An ambiguous name's question: "Which Lia?".
pub(crate) fn search_which(name: &str) -> String {
    template!("search.which", "name" => name)
}

/// The count once every page is in: "38 results".
pub(crate) fn search_results(count: u64) -> String {
    plural!("search.results", count)
}

/// The count while more pages exist: "100+ results".
pub(crate) fn search_results_more(count: u64) -> String {
    template!("search.results_more", "count" => count.to_string())
}

/// The scope covering the whole server.
pub(crate) fn search_scope_server() -> &'static str {
    text!("search.scope_server")
}

/// The scope covering the channel search was opened in.
pub(crate) fn search_scope_channel() -> &'static str {
    text!("search.scope_channel")
}

/// The scope choice's name, spoken.
pub(crate) fn search_scope() -> &'static str {
    text!("search.scope")
}

/// Nothing found: "No results for “kubernetes”".
pub(crate) fn search_none(query: &str) -> String {
    template!("search.none", "query" => query)
}

/// Clears the filters, keeping the words.
pub(crate) fn search_clear_filters() -> &'static str {
    text!("search.clear_filters")
}

/// A search that failed.
pub(crate) fn search_failed() -> &'static str {
    text!("search.failed")
}

/// Runs a failed search again.
pub(crate) fn search_retry() -> &'static str {
    text!("search.retry")
}

/// The field's ✕.
pub(crate) fn search_clear() -> &'static str {
    text!("search.clear")
}

/// Results loading, spoken.
pub(crate) fn search_loading() -> &'static str {
    text!("search.loading")
}

/// The results list, spoken.
pub(crate) fn search_results_list() -> &'static str {
    text!("search.results_list")
}

/// A result's spoken action: opening it in the conversation.
pub(crate) fn search_jump() -> &'static str {
    text!("search.jump")
}

/// The shortcut 🔍 names.
pub(crate) fn shortcut_search() -> &'static str {
    text!("shortcut.search")
}
