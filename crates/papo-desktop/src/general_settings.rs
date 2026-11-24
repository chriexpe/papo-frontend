//! The General settings (`docs/design.md` §8, General): the interface's
//! language (the system's, or one of the app's own) and whether a link to a
//! host that is not trusted asks first. Both are this device's. The trusted
//! domains are `links`' own list; Save media to comes with D7.
//!
//! Owns the values, their defaults and their keys in this device's
//! settings file. A change shows at once and is written off the UI thread;
//! a failed write takes the settings back to what the file holds. Putting
//! a language into effect is `strings`'.

use rune::prefs::Prefs;
use serde_json::json;

use crate::device_settings::{SettingsFile, Stored, Write};
use crate::strings;

/// "Language": `system`, or a locale's id (`en-US`, `pt-BR`).
const LANGUAGE_KEY: &str = "language";
/// The language key's value for the system's language.
const SYSTEM_LANGUAGE: &str = "system";
/// "Confirm external links".
const CONFIRM_LINKS_KEY: &str = "confirm_links";

/// Which language the interface speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Language {
    /// The supported language closest to the system's, else English.
    System,
    /// One of the app's locales, by its id.
    Locale(&'static str),
}

/// Everything the General page holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GeneralSettings {
    pub(crate) language: Language,
    /// A link to a host that is neither the server's nor trusted asks
    /// before it opens; off, every link opens at once.
    pub(crate) confirm_links: bool,
}

impl Default for GeneralSettings {
    /// The system's language, and links ask first.
    fn default() -> GeneralSettings {
        GeneralSettings {
            language: Language::System,
            confirm_links: true,
        }
    }
}

/// The General settings and where they are saved.
pub(crate) struct GeneralStore {
    stored: Stored<GeneralSettings>,
    file: SettingsFile,
}

impl Default for GeneralStore {
    /// The defaults, kept nowhere: before the settings file is read, and in
    /// tests.
    fn default() -> GeneralStore {
        GeneralStore::read(&Prefs::memory())
    }
}

impl GeneralStore {
    /// The settings in `prefs`, this device's settings file; a missing or
    /// unreadable value, or a language the app no longer has, takes its
    /// default.
    pub(crate) fn read(prefs: &Prefs) -> GeneralStore {
        let defaults = GeneralSettings::default();
        let language = prefs
            .get::<String>(LANGUAGE_KEY)
            .and_then(|id| strings::locale_id(&id))
            .map_or(defaults.language, Language::Locale);
        let settings = GeneralSettings {
            language,
            confirm_links: prefs
                .get(CONFIRM_LINKS_KEY)
                .unwrap_or(defaults.confirm_links),
        };
        GeneralStore {
            stored: Stored::new(settings),
            file: SettingsFile::of(prefs),
        }
    }

    /// The settings that apply now.
    pub(crate) fn settings(&self) -> &GeneralSettings {
        self.stored.shown()
    }

    /// Shows `settings` at once; the settings to write now, unless a write
    /// is running (they then follow it) or nothing changed.
    pub(crate) fn change(&mut self, settings: GeneralSettings) -> Option<GeneralSettings> {
        self.stored.change(settings)
    }

    /// The write of `settings` landed (`ok`) or did not, which takes them
    /// back to what the file holds; the settings to write next.
    pub(crate) fn written(
        &mut self,
        settings: GeneralSettings,
        ok: bool,
    ) -> Option<GeneralSettings> {
        self.stored.written(settings, ok)
    }

    /// The write that saves `settings`, both keys.
    pub(crate) fn write_of(&self, settings: &GeneralSettings) -> Write {
        let language = match settings.language {
            Language::System => SYSTEM_LANGUAGE,
            Language::Locale(id) => id,
        };
        self.file.write(vec![
            (LANGUAGE_KEY, json!(language)),
            (CONFIRM_LINKS_KEY, json!(settings.confirm_links)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{CONFIRM_LINKS_KEY, GeneralStore, LANGUAGE_KEY, Language};

    #[test]
    fn missing_settings_follow_the_system_and_ask_before_links() {
        let store = GeneralStore::read(&Prefs::memory());
        assert_eq!(store.settings().language, Language::System);
        assert!(store.settings().confirm_links);
    }

    #[test]
    fn a_language_the_app_has_is_read_and_one_it_lacks_is_the_system() {
        let mut prefs = Prefs::memory();
        prefs.set(LANGUAGE_KEY, "pt-BR").unwrap();
        assert_eq!(
            GeneralStore::read(&prefs).settings().language,
            Language::Locale("pt-BR")
        );
        prefs.set(LANGUAGE_KEY, "tlh").unwrap();
        assert_eq!(
            GeneralStore::read(&prefs).settings().language,
            Language::System
        );
        prefs.set(LANGUAGE_KEY, "system").unwrap();
        assert_eq!(
            GeneralStore::read(&prefs).settings().language,
            Language::System
        );
    }

    #[test]
    fn a_change_is_written_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut store = GeneralStore::read(&Prefs::at(&path));
        let mut changed = *store.settings();
        changed.language = Language::Locale("en-US");
        changed.confirm_links = false;
        let written = store.change(changed).expect("a write");
        store.write_of(&written).run().unwrap();
        assert_eq!(store.written(written, true), None);
        assert_eq!(Prefs::at(&path).get::<bool>(CONFIRM_LINKS_KEY), Some(false));
        assert_eq!(*GeneralStore::read(&Prefs::at(&path)).settings(), changed);
    }
}
