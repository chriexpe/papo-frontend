//! Settings › General › Embedded players (`docs/design.md` §15.7): whether provider players may
//! load at all (Play videos and music from other sites, on), and the sites allowed to play
//! without asking (Always allow adds one, Remove takes it off). This device's setting: the
//! backend holds no such thing. A change shows at once and is written off the UI thread; a
//! failed write takes the setting back to what the file holds.
//!
//! Owns the setting and its keys in the settings file. Asking, playing and each site's stored
//! data are the players' (`provider_players`, `web_view`).

use std::collections::BTreeSet;

use papo_sdk_ui::PlayerProvider;
use rune::prefs::Prefs;

use crate::device_settings::{SettingsFile, Stored, Write};

/// The settings file's key for the switch.
const ON_KEY: &str = "embedded_players";
/// The settings file's key for the allowed sites, by `PlayerProvider::key`.
const ALLOWED_KEY: &str = "embedded_players_allowed";

/// The setting's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlayerSettings {
    /// Play videos and music from other sites.
    pub(crate) on: bool,
    /// The sites that play without asking.
    pub(crate) allowed: BTreeSet<PlayerProvider>,
}

impl Default for PlayerSettings {
    fn default() -> PlayerSettings {
        PlayerSettings {
            on: true,
            allowed: BTreeSet::new(),
        }
    }
}

/// The setting, as shown and as saved.
pub(crate) struct EmbeddedPlayers {
    stored: Stored<PlayerSettings>,
    file: SettingsFile,
}

impl Default for EmbeddedPlayers {
    /// The defaults, kept nowhere: before the settings file is read, and in tests.
    fn default() -> EmbeddedPlayers {
        EmbeddedPlayers::read(&Prefs::memory())
    }
}

impl EmbeddedPlayers {
    /// The setting in `prefs`, this device's settings file. A site no longer on the SDK's
    /// allowlist is dropped.
    pub(crate) fn read(prefs: &Prefs) -> EmbeddedPlayers {
        let allowed = prefs
            .get::<Vec<String>>(ALLOWED_KEY)
            .unwrap_or_default()
            .iter()
            .filter_map(|key| PlayerProvider::from_key(key))
            .collect();
        let settings = PlayerSettings {
            on: prefs.get(ON_KEY).unwrap_or(true),
            allowed,
        };
        EmbeddedPlayers {
            stored: Stored::new(settings),
            file: SettingsFile::of(prefs),
        }
    }

    pub(crate) fn settings(&self) -> &PlayerSettings {
        self.stored.shown()
    }

    /// Whether `provider` plays without asking.
    pub(crate) fn allows(&self, provider: PlayerProvider) -> bool {
        self.settings().allowed.contains(&provider)
    }

    /// Always allow `provider`; the value to write now, unless a write runs or it was allowed
    /// already.
    pub(crate) fn allow(&mut self, provider: PlayerProvider) -> Option<PlayerSettings> {
        let mut settings = self.settings().clone();
        settings.allowed.insert(provider);
        self.stored.change(settings)
    }

    /// Shows `settings` at once (Try again after a failed save); the value to write now.
    pub(crate) fn replace(&mut self, settings: PlayerSettings) -> Option<PlayerSettings> {
        self.stored.change(settings)
    }

    /// The write of `settings` landed (`ok`) or did not, which takes the setting back to
    /// what the file holds; the value to write next.
    pub(crate) fn written(&mut self, settings: PlayerSettings, ok: bool) -> Option<PlayerSettings> {
        self.stored.written(settings, ok)
    }

    /// The write that saves `settings`.
    pub(crate) fn write_of(&self, settings: &PlayerSettings) -> Write {
        let allowed: Vec<&str> = settings.allowed.iter().map(|site| site.key()).collect();
        self.file.write(vec![
            (ON_KEY, serde_json::json!(settings.on)),
            (ALLOWED_KEY, serde_json::json!(allowed)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_on_and_ask_for_every_site_at_first() {
        let players = EmbeddedPlayers::default();
        assert!(players.settings().on);
        assert!(!players.allows(PlayerProvider::YouTube));
    }

    #[test]
    fn always_allow_and_remove_change_the_list_at_once() {
        let mut players = EmbeddedPlayers::default();
        let allowed = players.allow(PlayerProvider::YouTube).expect("a write");
        assert!(players.allows(PlayerProvider::YouTube));
        assert_eq!(players.written(allowed, true), None);
        assert_eq!(
            players.allow(PlayerProvider::YouTube),
            None,
            "already allowed"
        );
        let removed = players.replace(PlayerSettings::default()).expect("a write");
        assert!(!players.allows(PlayerProvider::YouTube));
        assert_eq!(players.written(removed, true), None);
    }

    #[test]
    fn the_file_keeps_the_switch_and_the_sites_and_drops_unknown_ones() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut players = EmbeddedPlayers::read(&Prefs::at(&path));
        let off = players
            .replace(PlayerSettings {
                on: false,
                allowed: BTreeSet::new(),
            })
            .unwrap();
        players.write_of(&off).run().unwrap();
        players.written(off, true);
        let allowed = players.allow(PlayerProvider::YouTube).unwrap();
        players.write_of(&allowed).run().unwrap();
        let mut prefs = Prefs::at(&path);
        prefs
            .set(ALLOWED_KEY, &serde_json::json!(["youtube", "vimeo"]))
            .unwrap();
        let read = EmbeddedPlayers::read(&Prefs::at(&path));
        assert!(!read.settings().on);
        assert_eq!(
            read.settings().allowed,
            BTreeSet::from([PlayerProvider::YouTube])
        );
    }
}
