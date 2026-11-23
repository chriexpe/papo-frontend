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

/// Shown when another Papo process holds the profile.
pub(crate) fn startup_profile_in_use() -> &'static str {
    text!("startup.profile_in_use")
}

/// Shown when the profile could not be opened.
pub(crate) fn startup_open_failed() -> &'static str {
    text!("startup.open_failed")
}

mod card;
mod composer;
mod links;
mod messages;
mod people;
mod reading;
mod replies;
mod servers;
mod sign_in;
mod timeline;

pub(crate) use card::*;
pub(crate) use composer::*;
pub(crate) use links::*;
pub(crate) use messages::*;
pub(crate) use people::*;
pub(crate) use reading::*;
pub(crate) use replies::*;
pub(crate) use servers::*;
pub(crate) use sign_in::*;
pub(crate) use timeline::*;

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
    fn the_typing_line_names_up_to_three_and_marks_where_each_name_is() {
        let line = |names: &[&str], phrase| {
            let (line, ranges) = super::typing_line(names, phrase);
            let named: Vec<String> = ranges
                .iter()
                .map(|range| line[range.clone()].to_owned())
                .collect();
            (line, named)
        };
        assert_eq!(line(&[], None).0, "");
        assert_eq!(
            line(&["Lia"], None),
            ("Lia is typing…".to_owned(), vec!["Lia".to_owned()])
        );
        assert_eq!(
            line(&["Yuki"], Some("is thinking very hard…")).0,
            "Yuki is thinking very hard…"
        );
        let (two, named) = line(&["Lia", "Yuki"], None);
        assert_eq!(two, "Lia and Yuki are typing…");
        assert_eq!(named, ["Lia", "Yuki"]);
        let (three, named) = line(&["Lia", "Yuki", "Caio"], Some("ignored"));
        assert_eq!(three, "Lia, Yuki and Caio are typing…");
        assert_eq!(named, ["Lia", "Yuki", "Caio"]);
        // A phrase from the server is words, never markers.
        assert_eq!(
            line(&["Yuki"], Some("\u{E001}\u{E002} hm")).0,
            "Yuki \u{E001}\u{E002} hm"
        );
        assert_eq!(
            line(&["A", "B", "C", "D"], None),
            ("Several people are typing…".to_owned(), Vec::new())
        );
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
