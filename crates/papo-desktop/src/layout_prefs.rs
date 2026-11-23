//! The window's remembered layout (`docs/design.md` §3): whether a direct
//! conversation shows its profile pane. Hidden at first; the last choice
//! lives in this device's settings file through `rune::prefs`, read at
//! launch and saved on each change. The members pane's choice joins it
//! with the roles session.
//!
//! Owns only the setting; the pane is `shell::profile_pane`'s.

use rune::prefs::Prefs;

use crate::window::APP_ID;

/// "Show the profile pane in a direct conversation".
const DM_PROFILE_KEY: &str = "dm_profile_pane";

/// The remembered layout, and where it is saved.
pub(crate) struct LayoutPrefs {
    dm_profile: bool,
    prefs: Prefs,
}

impl Default for LayoutPrefs {
    /// Hidden, kept nowhere: before the settings file is read, and in tests.
    fn default() -> LayoutPrefs {
        LayoutPrefs::read(Prefs::memory())
    }
}

impl LayoutPrefs {
    /// The layout in this user's settings file; a missing or unreadable
    /// value is hidden.
    pub(crate) fn load() -> LayoutPrefs {
        LayoutPrefs::read(Prefs::open(APP_ID))
    }

    fn read(prefs: Prefs) -> LayoutPrefs {
        LayoutPrefs {
            dm_profile: prefs.get(DM_PROFILE_KEY).unwrap_or(false),
            prefs,
        }
    }

    pub(crate) fn dm_profile(&self) -> bool {
        self.dm_profile
    }

    /// Shows or hides the profile pane, and saves the choice. A failed save
    /// keeps it for this run.
    pub(crate) fn set_dm_profile(&mut self, shown: bool) {
        self.dm_profile = shown;
        if let Err(error) = self.prefs.set(DM_PROFILE_KEY, &shown) {
            log::warn!("the profile pane's choice was not saved: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{DM_PROFILE_KEY, LayoutPrefs};

    #[test]
    fn the_profile_pane_is_hidden_until_shown_and_then_remembered() {
        let mut layout = LayoutPrefs::read(Prefs::memory());
        assert!(!layout.dm_profile());
        layout.set_dm_profile(true);
        assert_eq!(layout.prefs.get::<bool>(DM_PROFILE_KEY), Some(true));
        let again = LayoutPrefs::read(layout.prefs);
        assert!(again.dm_profile(), "read back next launch");
    }
}
