//! The device's reading settings (`docs/design.md` §8, Conversations): open
//! a channel at its newest message or where the reader left it, and whether
//! skipped blocks of unread messages show (the hourglass). They live in this
//! device's settings file through `rune::prefs`
//! (`$XDG_CONFIG_HOME/<app id>/prefs.json`), beside the trusted link domains,
//! and are read once at launch; the settings page that changes them comes
//! with the Appearance work (D6).

use rune::prefs::Prefs;

/// "Open at the newest message".
const OPEN_AT_NEWEST_KEY: &str = "open_at_newest";
/// "Unread hourglass".
const UNREAD_HOURGLASS_KEY: &str = "unread_hourglass";

/// How timelines open and what they show of skipped messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadingPrefs {
    /// Every timeline opens at its newest message; off, it returns where
    /// the reader left it.
    pub(crate) open_at_newest: bool,
    /// ⌛ and the skipped block's edges show, and ↓ can clear the block.
    pub(crate) unread_hourglass: bool,
}

impl Default for ReadingPrefs {
    /// The mock's defaults: return where you left, hourglass on.
    fn default() -> ReadingPrefs {
        ReadingPrefs {
            open_at_newest: false,
            unread_hourglass: true,
        }
    }
}

impl ReadingPrefs {
    /// The settings in `prefs`, this device's settings file; a missing or
    /// unreadable one takes the default.
    pub(crate) fn read(prefs: &Prefs) -> ReadingPrefs {
        let defaults = ReadingPrefs::default();
        ReadingPrefs {
            open_at_newest: prefs
                .get(OPEN_AT_NEWEST_KEY)
                .unwrap_or(defaults.open_at_newest),
            unread_hourglass: prefs
                .get(UNREAD_HOURGLASS_KEY)
                .unwrap_or(defaults.unread_hourglass),
        }
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{OPEN_AT_NEWEST_KEY, ReadingPrefs, UNREAD_HOURGLASS_KEY};

    #[test]
    fn missing_settings_take_the_defaults() {
        assert_eq!(
            ReadingPrefs::read(&Prefs::memory()),
            ReadingPrefs {
                open_at_newest: false,
                unread_hourglass: true
            }
        );
    }

    #[test]
    fn saved_settings_are_read_and_a_wrong_type_is_ignored() {
        let mut prefs = Prefs::memory();
        prefs.set(OPEN_AT_NEWEST_KEY, &true).unwrap();
        prefs.set(UNREAD_HOURGLASS_KEY, "no").unwrap();
        assert_eq!(
            ReadingPrefs::read(&prefs),
            ReadingPrefs {
                open_at_newest: true,
                unread_hourglass: true
            }
        );
    }
}
