//! Every user-visible string, in en-US (the source locale) and pt-BR.
//!
//! The catalogs live in `strings/`, one JSON file per locale; the build script
//! compiles them into the binary. Each string has an accessor here, and a key
//! that is not in the source catalog fails the build. Views take their text
//! from these accessors and never write a user-visible literal themselves.

mod catalog {
    include!(concat!(env!("OUT_DIR"), "/i18n.rs"));
}

use rune_i18n::{I18n, Key};

static I18N: I18n = I18n::new(&catalog::CATALOG);

/// The current locale's text for `$key`, resolved to an index at compile time.
macro_rules! text {
    ($key:literal) => {{
        const KEY: Key = Key::named(catalog::KEYS, $key);
        I18N.text(KEY)
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
