//! Every user-visible string, in en-US (the source locale) and pt-BR.
//!
//! The catalogs live in `strings/`, one JSON file per locale; the build script
//! compiles them into the binary. Each string has an accessor here, and a key
//! that is not in the source catalog fails the build. Views take their text
//! from these accessors and never write a user-visible literal themselves.

#[expect(
    clippy::unreadable_literal,
    reason = "rune-i18n's generated plural table writes its empty marker as a bare u32"
)]
mod catalog {
    include!(concat!(env!("OUT_DIR"), "/i18n.rs"));
}

use rune_i18n::{I18n, Key, PluralKey};

static I18N: I18n = I18n::new(&catalog::CATALOG);

/// The current locale's text for `$key`, resolved to an index at compile time.
macro_rules! text {
    ($key:literal) => {{
        const KEY: Key = Key::named(catalog::KEYS, $key);
        I18N.text(KEY)
    }};
}

/// The current locale's template for `$key` with each `{{$name}}` filled in.
macro_rules! template {
    ($key:literal, $($name:literal => $value:expr),+) => {{
        const KEY: Key = Key::named(catalog::KEYS, $key);
        I18N.format(KEY, &[$(($name, &$value)),+])
    }};
}

/// The current locale's form of `$key` for `$count`, with `{{count}}`
/// filled in.
macro_rules! plural {
    ($key:literal, $count:expr) => {{
        const KEY: PluralKey = PluralKey::named(catalog::PLURALS, $key);
        I18N.plural_format(KEY, $count, &[])
    }};
}

/// Switches to the supported locale closest to the system's languages, or to
/// en-US when none matches.
pub(crate) fn use_system_language() {
    let languages = rune_i18n::system_languages();
    let locale = I18N.negotiate(languages.iter().map(String::as_str));
    I18N.set_locale(locale);
}

/// The window title.
pub(crate) fn window_title() -> &'static str {
    text!("window.title")
}

/// What assistive technology announces for the message field.
pub(crate) fn composer_label() -> &'static str {
    text!("composer.label")
}

/// The hint shown in the empty message field.
pub(crate) fn composer_placeholder() -> &'static str {
    text!("composer.placeholder")
}

/// Shown when another Papo process holds the profile.
pub(crate) fn startup_profile_in_use() -> &'static str {
    text!("startup.profile_in_use")
}

/// Shown when the profile could not be opened.
pub(crate) fn startup_open_failed() -> &'static str {
    text!("startup.open_failed")
}

/// The server step's title.
pub(crate) fn add_server() -> &'static str {
    text!("sign_in.add_server")
}

/// The address field's label.
pub(crate) fn address() -> &'static str {
    text!("sign_in.address")
}

/// The example in the empty address field.
pub(crate) fn address_placeholder() -> &'static str {
    text!("sign_in.address_placeholder")
}

/// The server step's button.
pub(crate) fn connect() -> &'static str {
    text!("sign_in.connect")
}

/// The server step's button while the server is probed.
pub(crate) fn connecting() -> &'static str {
    text!("sign_in.connecting")
}

/// What assistive technology announces for the popup's ✕.
pub(crate) fn close() -> &'static str {
    text!("sign_in.close")
}

/// The server password field's label.
pub(crate) fn server_password() -> &'static str {
    text!("sign_in.server_password")
}

/// The server password step's button.
pub(crate) fn unlock() -> &'static str {
    text!("sign_in.unlock")
}

/// The server password step's button while it is checked.
pub(crate) fn unlocking() -> &'static str {
    text!("sign_in.unlocking")
}

/// The link back to the server step.
pub(crate) fn change_server() -> &'static str {
    text!("sign_in.change_server")
}

/// The username field's label.
pub(crate) fn username() -> &'static str {
    text!("sign_in.username")
}

/// The password field's label.
pub(crate) fn password() -> &'static str {
    text!("sign_in.password")
}

/// The sign-in step's button.
pub(crate) fn sign_in() -> &'static str {
    text!("sign_in.sign_in")
}

/// The sign-in step's button while it signs in.
pub(crate) fn signing_in() -> &'static str {
    text!("sign_in.signing_in")
}

/// The create-account step's button, and the link to it.
pub(crate) fn create_account() -> &'static str {
    text!("sign_in.create_account")
}

/// The create-account step's button while it registers and signs in.
pub(crate) fn creating_account() -> &'static str {
    text!("sign_in.creating_account")
}

/// The link from create account back to sign in.
pub(crate) fn have_account() -> &'static str {
    text!("sign_in.have_account")
}

/// The new password's rules, under its field.
pub(crate) fn password_hint() -> &'static str {
    text!("sign_in.password_hint")
}

/// The button that shows a password's characters.
pub(crate) fn show() -> &'static str {
    text!("sign_in.show")
}

/// The button that hides a password's characters.
pub(crate) fn hide() -> &'static str {
    text!("sign_in.hide")
}

/// What assistive technology announces for Show.
pub(crate) fn show_password() -> &'static str {
    text!("sign_in.show_password")
}

/// What assistive technology announces for Hide.
pub(crate) fn hide_password() -> &'static str {
    text!("sign_in.hide_password")
}

/// A feedback line's action that sends the step again.
pub(crate) fn try_again() -> &'static str {
    text!("sign_in.try_again")
}

/// The already-added line's action.
pub(crate) fn open() -> &'static str {
    text!("sign_in.open")
}

/// The address names a scheme other than HTTP or HTTPS.
pub(crate) fn error_scheme() -> &'static str {
    text!("sign_in.error.scheme")
}

/// The address holds a username or password.
pub(crate) fn error_userinfo() -> &'static str {
    text!("sign_in.error.userinfo")
}

/// The address holds a `?` query.
pub(crate) fn error_query() -> &'static str {
    text!("sign_in.error.query")
}

/// The address holds a `#` fragment.
pub(crate) fn error_fragment() -> &'static str {
    text!("sign_in.error.fragment")
}

/// The address is not an address.
pub(crate) fn error_invalid() -> &'static str {
    text!("sign_in.error.invalid")
}

/// Plain HTTP to a host that is not local.
pub(crate) fn error_unencrypted() -> &'static str {
    text!("sign_in.error.unencrypted")
}

/// The probe got no usable answer.
pub(crate) fn error_unreachable() -> &'static str {
    text!("sign_in.error.unreachable")
}

/// A Papo server that has not been set up.
pub(crate) fn error_not_set_up() -> &'static str {
    text!("sign_in.error.not_set_up")
}

/// Something that is not a Papo server answered.
pub(crate) fn error_not_papo() -> &'static str {
    text!("sign_in.error.not_papo")
}

/// The server rejected its password.
pub(crate) fn error_wrong_server_password() -> &'static str {
    text!("sign_in.error.wrong_server_password")
}

/// The server rejected the username or password.
pub(crate) fn error_wrong_credentials() -> &'static str {
    text!("sign_in.error.wrong_credentials")
}

/// The account is banned.
pub(crate) fn error_banned() -> &'static str {
    text!("sign_in.error.banned")
}

/// Too many attempts.
pub(crate) fn error_rate_limited() -> &'static str {
    text!("sign_in.error.rate_limited")
}

/// An account step got no usable answer.
pub(crate) fn error_no_response() -> &'static str {
    text!("sign_in.error.no_response")
}

/// Create account asked for a taken username.
pub(crate) fn error_name_taken() -> &'static str {
    text!("sign_in.error.name_taken")
}

/// The keyring refused to keep the credentials.
pub(crate) fn error_keyring() -> &'static str {
    text!("sign_in.error.keyring")
}

/// A failure no other line names.
pub(crate) fn error_failed() -> &'static str {
    text!("sign_in.error.failed")
}

/// The server password's access lapsed during sign-in.
pub(crate) fn notice_access_expired() -> &'static str {
    text!("sign_in.notice.access_expired")
}

/// A saved account must sign in again.
pub(crate) fn notice_session_expired() -> &'static str {
    text!("sign_in.notice.session_expired")
}

/// What assistive technology announces for a pill's ✕.
pub(crate) fn dismiss() -> &'static str {
    text!("toast.dismiss")
}

/// The already-added line, naming the saved server.
pub(crate) fn error_already_added(server: &str) -> String {
    template!("sign_in.error.already_added", "server" => server)
}

/// The pill shown once signed in, naming the server.
pub(crate) fn connected_to(server: &str) -> String {
    template!("toast.connected", "server" => server)
}

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    fn keys(catalog: &str) -> BTreeSet<String> {
        let Value::Object(messages) = serde_json::from_str(catalog).unwrap() else {
            panic!("a catalog is a JSON object");
        };
        messages.into_iter().map(|(key, _)| key).collect()
    }

    #[test]
    fn plurals_pick_the_form_for_the_count() {
        // Tests run in the source locale, en-US; nothing switches it.
        assert_eq!(super::badge_mentions(1), "1 mention");
        assert_eq!(super::badge_mentions(2), "2 mentions");
    }

    #[test]
    fn every_locale_translates_every_string() {
        let source = keys(include_str!("../strings/papo/en-US.json"));
        let portuguese = keys(include_str!("../strings/papo/pt-BR.json"));
        assert_eq!(source, portuguese);
    }

    #[test]
    fn portuguese_is_offered_to_brazilian_systems() {
        let locale = super::I18N.negotiate(["pt-BR", "en-US"]);
        assert_eq!(super::I18N.locales()[locale].id(), "pt-BR");
    }
}
