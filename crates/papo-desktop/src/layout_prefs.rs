//! The window's remembered layout (`docs/design.md` §3): whether a direct
//! conversation shows its profile pane. Hidden at first; the last choice
//! lives in this device's settings file through `rune::prefs`, read at
//! launch and saved on each change. The members pane's choice joins it
//! with the roles session.
//!
//! Owns only the setting; the pane is `shell::profile_pane`'s.

use rune::prefs::Prefs;

use crate::device_settings::{Remembered, SettingsFile, Write};

/// "Show the profile pane in a direct conversation".
const DM_PROFILE_KEY: &str = "dm_profile_pane";

/// The remembered layout, and where it is saved.
pub(crate) struct LayoutPrefs {
    dm_profile: bool,
    remembered: Remembered,
}

impl Default for LayoutPrefs {
    /// Hidden, kept nowhere: before the settings file is read, and in tests.
    fn default() -> LayoutPrefs {
        LayoutPrefs::read(&Prefs::memory())
    }
}

impl LayoutPrefs {
    /// The layout in `prefs`, this device's settings file; a missing or
    /// unreadable value is hidden.
    pub(crate) fn read(prefs: &Prefs) -> LayoutPrefs {
        LayoutPrefs {
            dm_profile: prefs.get(DM_PROFILE_KEY).unwrap_or(false),
            remembered: Remembered::new(DM_PROFILE_KEY, SettingsFile::of(prefs)),
        }
    }

    pub(crate) fn dm_profile(&self) -> bool {
        self.dm_profile
    }

    /// Shows or hides the profile pane; the write that saves the choice, unless
    /// one is running (it then follows). A failed save keeps it for this run.
    pub(crate) fn set_dm_profile(&mut self, shown: bool) -> Option<Write> {
        self.dm_profile = shown;
        self.remembered.remember(serde_json::Value::Bool(shown))
    }

    /// The write waiting behind the running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.remembered.pending()
    }

    /// The running write landed, or did not; the one to run next.
    pub(crate) fn written(&mut self, ok: bool) -> Option<Write> {
        self.remembered.written(ok)
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{DM_PROFILE_KEY, LayoutPrefs};

    #[test]
    fn the_profile_pane_is_hidden_until_shown_and_then_remembered() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut layout = LayoutPrefs::read(&Prefs::at(&path));
        assert!(!layout.dm_profile());
        layout.set_dm_profile(true).expect("a write").run().unwrap();
        assert_eq!(layout.written(true), None);
        assert_eq!(Prefs::at(&path).get::<bool>(DM_PROFILE_KEY), Some(true));
        let again = LayoutPrefs::read(&Prefs::at(&path));
        assert!(again.dm_profile(), "read back next launch");
    }
}
