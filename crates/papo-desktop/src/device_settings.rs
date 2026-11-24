//! Writing this device's settings file (`rune::prefs`,
//! `$XDG_CONFIG_HOME/<app id>/prefs.json`), where every part of the app
//! keeps its values under its own keys: Appearance, the Conversations and
//! General settings, the trusted domains, the remembered layout and the
//! recent jumps.
//!
//! `rune::prefs` rewrites the whole file on each `set`, from the copy it
//! read. Two writes that overlap would each put back the other's keys as
//! they were, so every write goes through [`Write::run`]: it takes one lock
//! for the whole program, reads the file afresh and sets its keys. Each
//! write is numbered when it is made, and a key takes a write only if it is
//! newer than the last that key took, so writes that run out of order (at
//! exit, say) never put an older value back. The lock holds file I/O on
//! purpose: it guards nothing but the file. The shell runs writes as SDK
//! `finish` jobs off the UI thread (`Shell::run_write`); only before the
//! SDK client opens, or if it cannot, does a write run on the UI thread.
//!
//! Owns the file's path, the lock and the one-write-at-a-time queue each
//! owner keeps (`saving`); what each key holds is its owner's.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use rune::prefs::Prefs;
use serde_json::Value;

use crate::window::APP_ID;

mod saving;

pub(crate) use saving::{Remembered, Stored};

/// Taken by every write, for its whole read and write of the file; it holds
/// the number of the last write each key of each file took.
static FILE_LOCK: LazyLock<Mutex<HashMap<(PathBuf, &'static str), u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The next write's number: writes are numbered as they are made.
static NEXT_WRITE: AtomicU64 = AtomicU64::new(1);

/// A write to the settings file that did not land; the setting keeps its
/// earlier value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

/// This device's settings file, read once at launch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SettingsFile {
    /// None for settings kept nowhere: before the file is read, and in
    /// tests.
    path: Option<PathBuf>,
}

impl SettingsFile {
    /// What this user's settings file holds now, read once at launch for
    /// every owner; an unreadable file holds nothing.
    pub(crate) fn read() -> Prefs {
        Prefs::open(APP_ID)
    }

    /// The file `prefs` reads.
    pub(crate) fn of(prefs: &Prefs) -> SettingsFile {
        SettingsFile {
            path: prefs.path().map(std::path::Path::to_path_buf),
        }
    }

    /// A write of `entries`, each key set to its value.
    pub(crate) fn write(&self, entries: Vec<(&'static str, Value)>) -> Write {
        Write {
            path: self.path.clone(),
            entries,
            number: NEXT_WRITE.fetch_add(1, Ordering::Relaxed),
        }
    }
}

/// One write of some keys, which may run on any thread.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Write {
    path: Option<PathBuf>,
    entries: Vec<(&'static str, Value)>,
    /// When it was made, among all writes.
    number: u64,
}

impl Write {
    /// Sets the keys, blocking, under the file's lock: the file is read
    /// afresh first, so what another part of the app saved stays. Settings
    /// kept nowhere have nothing to write.
    pub(crate) fn run(&self) -> Result<(), NotSaved> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        // A write that panicked held the lock over a file it either wrote
        // whole or not at all (`rune::prefs` replaces the file atomically).
        let mut taken = FILE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut prefs = Prefs::at(path.clone());
        for (key, value) in &self.entries {
            let slot = (path.clone(), *key);
            if taken.get(&slot).is_some_and(|last| *last > self.number) {
                // A newer write already set this key.
                continue;
            }
            taken.insert(slot, self.number);
            prefs.set(key, value).map_err(|error| {
                log::warn!("the setting {key} was not saved: {error}");
                NotSaved
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;
    use serde_json::json;

    use super::SettingsFile;

    #[test]
    fn writes_of_different_keys_keep_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let file = SettingsFile::of(&Prefs::at(&path));
        // Both made from the same launch-time file: the second must not
        // put the first's key back as it was.
        let links = file.write(vec![("trusted_domains", json!(["example.org"]))]);
        let layout = file.write(vec![("dm_profile_pane", json!(true))]);
        links.run().unwrap();
        layout.run().unwrap();
        let on_disk = Prefs::at(&path);
        assert_eq!(
            on_disk.get::<Vec<String>>("trusted_domains"),
            Some(vec!["example.org".to_owned()])
        );
        assert_eq!(on_disk.get::<bool>("dm_profile_pane"), Some(true));
    }

    #[test]
    fn writes_from_many_threads_lose_no_key() {
        const KEYS: [&str; 8] = ["k0", "k1", "k2", "k3", "k4", "k5", "k6", "k7"];
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let file = SettingsFile::of(&Prefs::at(&path));
        std::thread::scope(|scope| {
            for (n, key) in KEYS.into_iter().enumerate() {
                let write = file.write(vec![(key, json!(n))]);
                scope.spawn(move || write.run().unwrap());
            }
        });
        let on_disk = Prefs::at(&path);
        for (n, key) in KEYS.into_iter().enumerate() {
            assert_eq!(on_disk.get::<usize>(key), Some(n), "{key}");
        }
    }

    #[test]
    fn an_older_write_that_runs_last_never_puts_its_value_back() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let file = SettingsFile::of(&Prefs::at(&path));
        let older = file.write(vec![("theme", json!("light"))]);
        let newer = file.write(vec![("theme", json!("oled"))]);
        newer.run().unwrap();
        older.run().unwrap();
        assert_eq!(
            Prefs::at(&path).get::<String>("theme").as_deref(),
            Some("oled")
        );
    }

    #[test]
    fn settings_kept_nowhere_write_nothing() {
        let write = SettingsFile::default().write(vec![("anything", json!(1))]);
        assert!(write.run().is_ok());
    }
}
