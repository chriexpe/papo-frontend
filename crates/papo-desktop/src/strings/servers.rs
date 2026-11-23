//! The rail's, the channel pane's and the server pill's strings, and the
//! badges'.

use super::{I18N, Key, PluralKey, catalog};

/// What assistive technology announces for the server rail.
pub(crate) fn rail_label() -> &'static str {
    text!("rail.label")
}

/// What assistive technology announces for the rail's +.
pub(crate) fn rail_add_server() -> &'static str {
    text!("rail.add_server")
}

/// How the keyboard moves a server in the rail.
pub(crate) fn rail_move_hint() -> &'static str {
    text!("rail.move_hint")
}

/// What assistive technology announces for the channel list.
pub(crate) fn channels_label() -> &'static str {
    text!("channels.label")
}

/// The text channels' group label, in capitals as the list shows it.
pub(crate) fn text_channels() -> &'static str {
    text!("channels.text")
}

/// The voice channels' group label, in capitals as the list shows it.
pub(crate) fn voice_channels() -> &'static str {
    text!("channels.voice")
}

/// What assistive technology announces for the server card's ⚙.
pub(crate) fn server_options() -> &'static str {
    text!("server_card.options")
}

/// The server pill's ⇄, which opens the accounts on the server.
pub(crate) fn switch_account() -> &'static str {
    text!("server_card.switch_account")
}

/// The caption over a server's accounts, naming the server as given (the
/// caller writes it in capitals, as the catalog writes the rest).
pub(crate) fn accounts_on(server: &str) -> String {
    template!("accounts.title", "server" => server)
}

/// An account's row in the switch, for assistive technology.
pub(crate) fn account_row(name: &str, username: &str) -> String {
    template!("accounts.row", "name" => name, "username" => username)
}

/// The switch's last row: another account on this server.
pub(crate) fn add_account() -> &'static str {
    text!("accounts.add")
}

/// The server menu's item that asks to remove it.
pub(crate) fn remove_server() -> &'static str {
    text!("server_menu.remove")
}

/// What removing a server does, in its confirmation.
pub(crate) fn remove_body() -> &'static str {
    text!("remove.body")
}

/// Keeps the server.
pub(crate) fn remove_cancel() -> &'static str {
    text!("remove.cancel")
}

/// Removes the server.
pub(crate) fn remove_confirm() -> &'static str {
    text!("remove.confirm")
}

/// The server card's line naming the server's owner.
pub(crate) fn server_owner(name: &str) -> String {
    template!("server_card.owner", "name" => name)
}

/// An account's username as the account pill shows it.
pub(crate) fn account_handle(username: &str) -> String {
    template!("account.handle", "username" => username)
}

/// The removal confirmation's question, naming the server.
pub(crate) fn remove_title(server: &str) -> String {
    template!("remove.title", "server" => server)
}

/// What assistive technology adds for an unread channel or server.
pub(crate) fn badge_unread() -> &'static str {
    text!("badge.unread")
}

/// "3 mentions", for assistive technology.
pub(crate) fn badge_mentions(count: u64) -> String {
    plural!("badge.mentions", count)
}

/// "at least 3 mentions", when the SDK knows only a lower bound.
pub(crate) fn badge_mentions_at_least(count: u64) -> String {
    plural!("badge.mentions_at_least", count)
}

/// A count shown as a lower bound: "3+", and "99+" past the cap.
pub(crate) fn badge_count_more(count: u64) -> String {
    template!("badge.count_more", "count" => count)
}

/// A row's or tile's name followed by what its badge says.
pub(crate) fn badge_labelled(name: &str, badge: &str) -> String {
    template!("badge.labelled", "name" => name, "badge" => badge)
}

/// Direct's accounts list, grown from its ⇄: every account on every server.
pub(crate) fn direct_accounts() -> &'static str {
    text!("accounts.direct")
}
