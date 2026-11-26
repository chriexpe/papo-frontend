//! Device-scoped notification settings (`docs/design.md` §8, Notifications):
//! Tray counter ("This device only"). Account-scoped notification settings
//! (`enabled`, `mentions`, `message_preview`, `sound`) live in the SDK.

use rune::prefs::Prefs;
use serde_json::{Value, json};

use crate::device_settings::{SettingsFile, Stored, Write};

const TRAY_COUNTER_KEY: &str = "tray_counter";

/// Device-scoped notification preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotificationPrefs {
    /// Whether the system tray icon shows the unread/mention counter.
    pub(crate) tray_counter: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self { tray_counter: true }
    }
}

/// The device notification preferences and where they are saved.
pub(crate) struct NotificationStore {
    stored: Stored<NotificationPrefs>,
    file: SettingsFile,
}

impl Default for NotificationStore {
    fn default() -> Self {
        Self::read(&Prefs::memory())
    }
}

impl NotificationStore {
    /// Reads preferences from `prefs`; missing keys take their defaults.
    pub(crate) fn read(prefs: &Prefs) -> Self {
        let defaults = NotificationPrefs::default();
        let settings = NotificationPrefs {
            tray_counter: prefs.get(TRAY_COUNTER_KEY).unwrap_or(defaults.tray_counter),
        };
        Self {
            stored: Stored::new(settings),
            file: SettingsFile::of(prefs),
        }
    }

    /// The settings in effect now.
    pub(crate) fn settings(&self) -> &NotificationPrefs {
        self.stored.shown()
    }

    /// Applies `settings` at once; returns the value to write if none is in flight.
    pub(crate) fn change(&mut self, settings: NotificationPrefs) -> Option<NotificationPrefs> {
        self.stored.change(settings)
    }

    /// Reports the write outcome and returns any queued change to write next.
    pub(crate) fn written(
        &mut self,
        settings: NotificationPrefs,
        ok: bool,
    ) -> Option<NotificationPrefs> {
        self.stored.written(settings, ok)
    }

    /// The write waiting behind a running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.stored
            .pending()
            .copied()
            .map(|settings| self.write_of(settings))
    }

    /// The write that saves `settings`.
    pub(crate) fn write_of(&self, settings: NotificationPrefs) -> Write {
        let saved = entries(*self.stored.saved());
        let changed = entries(settings)
            .into_iter()
            .filter(|entry| !saved.contains(entry))
            .collect();
        self.file.write(changed)
    }
}

fn entries(settings: NotificationPrefs) -> Vec<(&'static str, Value)> {
    vec![(TRAY_COUNTER_KEY, json!(settings.tray_counter))]
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    use super::{NotificationPrefs, NotificationStore, TRAY_COUNTER_KEY};

    #[test]
    fn tray_counter_defaults_on_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let mut store = NotificationStore::read(&Prefs::at(&path));
        assert!(store.settings().tray_counter);
        let next = NotificationPrefs {
            tray_counter: false,
        };
        let to_write = store.change(next).expect("pending write");
        store.write_of(to_write).run().unwrap();
        assert_eq!(store.written(to_write, true), None);
        assert_eq!(Prefs::at(&path).get::<bool>(TRAY_COUNTER_KEY), Some(false));
        assert!(
            !NotificationStore::read(&Prefs::at(&path))
                .settings()
                .tray_counter
        );
    }
}
