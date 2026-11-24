//! The device's writing settings (`docs/design.md` §8, Conversations, "When
//! writing"): "Enter to send" (on, Enter sends and Shift+Enter adds a line;
//! off, the other way round) and "Mention when replying" (where a reply's @
//! switch starts). It lives in this device's settings file
//! through `rune::prefs`, beside the reading settings, and is read once at
//! launch; the settings page that changes it comes with the Appearance work
//! (D6).

use rune::prefs::Prefs;

/// "Enter to send".
const ENTER_SENDS_KEY: &str = "enter_sends";
/// "Mention when replying".
const MENTION_REPLY_KEY: &str = "mention_when_replying";

/// How the composer takes Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WritingPrefs {
    /// Enter sends and Shift+Enter adds a line; off, the reverse.
    pub(crate) enter_sends: bool,
    /// A reply's @ switch starts on.
    pub(crate) mention_when_replying: bool,
}

impl Default for WritingPrefs {
    /// The mock's defaults: Enter sends, and replies mention.
    fn default() -> WritingPrefs {
        WritingPrefs {
            enter_sends: true,
            mention_when_replying: true,
        }
    }
}

impl WritingPrefs {
    /// The settings in `prefs`, this device's settings file; a missing or
    /// unreadable one takes the default.
    pub(crate) fn read(prefs: &Prefs) -> WritingPrefs {
        let default = WritingPrefs::default();
        WritingPrefs {
            enter_sends: prefs.get(ENTER_SENDS_KEY).unwrap_or(default.enter_sends),
            mention_when_replying: prefs
                .get(MENTION_REPLY_KEY)
                .unwrap_or(default.mention_when_replying),
        }
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{ENTER_SENDS_KEY, MENTION_REPLY_KEY, WritingPrefs};

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

    #[test]
    fn replies_mention_unless_the_setting_says_otherwise() {
        assert!(WritingPrefs::read(&Prefs::memory()).mention_when_replying);
        let mut prefs = Prefs::memory();
        prefs.set(MENTION_REPLY_KEY, &false).unwrap();
        assert!(!WritingPrefs::read(&prefs).mention_when_replying);
    }
}
