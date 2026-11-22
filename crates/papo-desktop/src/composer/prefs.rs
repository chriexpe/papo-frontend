//! The device's writing setting (`docs/design.md` §8, Conversations, "When
//! writing"): "Enter to send". On, Enter sends and Shift+Enter adds a line;
//! off, the other way round. It lives in this device's settings file
//! through `rune::prefs`, beside the reading settings, and is read once at
//! launch; the settings page that changes it comes with the Appearance work
//! (D6).

use rune::prefs::Prefs;

use crate::window::APP_ID;

/// "Enter to send".
const ENTER_SENDS_KEY: &str = "enter_sends";

/// How the composer takes Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WritingPrefs {
    /// Enter sends and Shift+Enter adds a line; off, the reverse.
    pub(crate) enter_sends: bool,
}

impl Default for WritingPrefs {
    /// The mock's default: Enter sends.
    fn default() -> WritingPrefs {
        WritingPrefs { enter_sends: true }
    }
}

impl WritingPrefs {
    /// The setting in this user's settings file; a missing or unreadable
    /// one takes the default.
    pub(crate) fn load() -> WritingPrefs {
        WritingPrefs::read(&Prefs::open(APP_ID))
    }

    fn read(prefs: &Prefs) -> WritingPrefs {
        WritingPrefs {
            enter_sends: prefs
                .get(ENTER_SENDS_KEY)
                .unwrap_or(WritingPrefs::default().enter_sends),
        }
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{ENTER_SENDS_KEY, WritingPrefs};

    #[test]
    fn enter_sends_unless_the_setting_says_otherwise() {
        assert!(WritingPrefs::read(&Prefs::memory()).enter_sends);
        let mut prefs = Prefs::memory();
        prefs.set(ENTER_SENDS_KEY, &false).unwrap();
        assert!(!WritingPrefs::read(&prefs).enter_sends);
        prefs.set(ENTER_SENDS_KEY, "no").unwrap();
        assert!(
            WritingPrefs::read(&prefs).enter_sends,
            "a wrong type is ignored"
        );
    }
}
