//! The saves of received media (`docs/design.md` §14.6, §14.7): which attachment saves, how
//! far it came, where it went, and the pill a save from the menu or the viewer ends with
//! ("Saved praia.jpg", Show in folder, for 4 s). A file card shows its own save instead.
//!
//! Owns the state; the SDK writes the file (`MediaService::save_attachment`), the shell starts
//! saves and follows them (`shell::saving`), and the views read [`SaveView`]s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use papo_sdk::{AccountId, SaveState};

use crate::media_store::MediaKey;

/// How long the saved pill stays, in seconds.
pub(crate) const SAVED_PILL_FOR_S: f64 = 4.0;

/// What a view shows of an attachment's save.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SaveView {
    /// Writing: `saved_bytes` of `total_bytes` (when known).
    Saving {
        saved_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// Saved at this path.
    Saved(PathBuf),
    /// It failed; Save tries again.
    Failed,
}

/// The pill a save ends with: the file's name and where it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SavedPill {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// Moves with every pill, so a timer for an earlier one does not close it.
    pub(crate) serial: u64,
}

/// One running save.
struct Running {
    /// Stops it; it holds the save's handle, so the save runs while this is kept.
    cancel: Box<dyn Fn()>,
    /// Tells this save's updates from an earlier one's for the same attachment.
    serial: u64,
    /// It ends with the saved pill.
    with_pill: bool,
}

/// Every save this run started.
#[derive(Default)]
pub(crate) struct Saves {
    views: Arc<HashMap<MediaKey, SaveView>>,
    running: HashMap<MediaKey, Running>,
    pill: Option<SavedPill>,
    last_serial: u64,
}

impl Saves {
    /// What the views read.
    pub(crate) fn views(&self) -> Arc<HashMap<MediaKey, SaveView>> {
        Arc::clone(&self.views)
    }

    /// Whether `key` is saving now.
    #[cfg(test)]
    pub(crate) fn is_saving(&self, key: MediaKey) -> bool {
        self.running.contains_key(&key)
    }

    /// `key` started saving; `cancel` stops it (and keeps its handle); `with_pill` when it ends
    /// with the saved pill. Returns its serial, which its updates carry. An earlier save of
    /// `key` is cancelled.
    pub(crate) fn started(
        &mut self,
        key: MediaKey,
        cancel: impl Fn() + 'static,
        with_pill: bool,
    ) -> u64 {
        if let Some(earlier) = self.running.remove(&key) {
            (earlier.cancel)();
        }
        self.last_serial += 1;
        let serial = self.last_serial;
        self.running.insert(
            key,
            Running {
                cancel: Box::new(cancel),
                serial,
                with_pill,
            },
        );
        Arc::make_mut(&mut self.views).insert(
            key,
            SaveView::Saving {
                saved_bytes: 0,
                total_bytes: None,
            },
        );
        serial
    }

    /// Save `serial` of `key` is at `state`, for a file named `name`. Returns whether a view
    /// changed. A save that ended forgets its handle; a cancelled one shows as it was before.
    pub(crate) fn updated(
        &mut self,
        key: MediaKey,
        serial: u64,
        state: &SaveState,
        name: &str,
    ) -> bool {
        let Some(running) = self
            .running
            .get(&key)
            .filter(|running| running.serial == serial)
        else {
            return false;
        };
        let with_pill = running.with_pill;
        let view = match state {
            SaveState::Saving(progress) => Some(SaveView::Saving {
                saved_bytes: progress.saved_bytes,
                total_bytes: progress.total_bytes,
            }),
            SaveState::Saved(path) => {
                if with_pill {
                    self.last_serial += 1;
                    self.pill = Some(SavedPill {
                        name: name.to_owned(),
                        path: path.clone(),
                        serial: self.last_serial,
                    });
                }
                Some(SaveView::Saved(path.clone()))
            }
            SaveState::Failed(papo_sdk::SaveFailure::Cancelled) => None,
            SaveState::Failed(_) => Some(SaveView::Failed),
        };
        if state.is_finished() {
            self.running.remove(&key);
        }
        let views = Arc::make_mut(&mut self.views);
        match view {
            Some(view) if views.get(&key) == Some(&view) => false,
            Some(view) => {
                views.insert(key, view);
                true
            }
            None => views.remove(&key).is_some(),
        }
    }

    /// ✕ on a saving file: the save stops and the card shows Save again.
    pub(crate) fn cancel(&mut self, key: MediaKey) -> bool {
        let Some(running) = self.running.remove(&key) else {
            return false;
        };
        (running.cancel)();
        Arc::make_mut(&mut self.views).remove(&key).is_some()
    }

    /// Only accounts for which `kept` holds stay: the others' saves stop and their views go.
    pub(crate) fn keep_accounts(&mut self, kept: impl Fn(AccountId) -> bool) {
        self.running.retain(|key, running| {
            let keep = kept(key.account);
            if !keep {
                (running.cancel)();
            }
            keep
        });
        if self.views.keys().any(|key| !kept(key.account)) {
            Arc::make_mut(&mut self.views).retain(|key, _| kept(key.account));
        }
    }

    /// The pill showing now.
    pub(crate) fn pill(&self) -> Option<&SavedPill> {
        self.pill.as_ref()
    }

    /// Pill `serial`'s time is up (or it was closed): it goes, unless a later one replaced it.
    pub(crate) fn pill_gone(&mut self, serial: u64) -> bool {
        if self.pill.as_ref().is_some_and(|pill| pill.serial == serial) {
            self.pill = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    //! A save's views, its end and the pill, from synthetic states.

    use super::*;
    use papo_sdk::{AccountId, SaveFailure, SaveProgress, ServerAddress, Uuid};
    use std::cell::Cell;
    use std::rc::Rc;

    fn key() -> MediaKey {
        MediaKey {
            account: AccountId::new(
                ServerAddress::parse("https://lumen.example.invalid")
                    .unwrap()
                    .id(),
                Uuid::from_u128(1),
            ),
            attachment: Uuid::from_u128(2),
        }
    }

    fn saving(saved_bytes: u64) -> SaveState {
        SaveState::Saving(SaveProgress {
            saved_bytes,
            total_bytes: Some(10),
        })
    }

    #[test]
    fn a_save_shows_its_progress_then_where_it_went() {
        let mut saves = Saves::default();
        let serial = saves.started(key(), || {}, false);
        assert!(saves.updated(key(), serial, &saving(4), "a.pdf"));
        assert_eq!(
            saves.views().get(&key()),
            Some(&SaveView::Saving {
                saved_bytes: 4,
                total_bytes: Some(10)
            })
        );
        let path = PathBuf::from("/tmp/a.pdf");
        assert!(saves.updated(key(), serial, &SaveState::Saved(path.clone()), "a.pdf"));
        assert_eq!(saves.views().get(&key()), Some(&SaveView::Saved(path)));
        assert!(!saves.is_saving(key()));
        assert!(saves.pill().is_none(), "a card shows its own save");
    }

    #[test]
    fn an_earlier_save_s_updates_are_dropped_and_it_is_cancelled() {
        let cancelled = Rc::new(Cell::new(false));
        let mut saves = Saves::default();
        let first = {
            let cancelled = Rc::clone(&cancelled);
            saves.started(key(), move || cancelled.set(true), false)
        };
        let second = saves.started(key(), || {}, false);
        assert!(cancelled.get());
        assert!(!saves.updated(key(), first, &saving(9), "a.pdf"));
        assert!(saves.updated(key(), second, &saving(1), "a.pdf"));
    }

    #[test]
    fn a_failed_save_shows_and_a_cancelled_one_does_not() {
        let mut saves = Saves::default();
        let serial = saves.started(key(), || {}, false);
        saves.updated(
            key(),
            serial,
            &SaveState::Failed(SaveFailure::Unreachable),
            "a",
        );
        assert_eq!(saves.views().get(&key()), Some(&SaveView::Failed));
        let serial = saves.started(key(), || {}, false);
        saves.updated(
            key(),
            serial,
            &SaveState::Failed(SaveFailure::Cancelled),
            "a",
        );
        assert_eq!(saves.views().get(&key()), None);
    }

    #[test]
    fn a_save_from_the_menu_ends_with_the_pill_until_its_time_is_up() {
        let mut saves = Saves::default();
        let serial = saves.started(key(), || {}, true);
        saves.updated(
            key(),
            serial,
            &SaveState::Saved(PathBuf::from("/d/praia.jpg")),
            "praia.jpg",
        );
        let pill = saves.pill().cloned().expect("a pill");
        assert_eq!(pill.name, "praia.jpg");
        assert!(!saves.pill_gone(pill.serial + 1), "another pill's timer");
        assert!(saves.pill_gone(pill.serial));
        assert!(saves.pill().is_none());
    }

    #[test]
    fn cancel_stops_the_save_and_shows_save_again() {
        let cancelled = Rc::new(Cell::new(false));
        let mut saves = Saves::default();
        {
            let cancelled = Rc::clone(&cancelled);
            saves.started(key(), move || cancelled.set(true), false);
        }
        assert!(saves.cancel(key()));
        assert!(cancelled.get());
        assert_eq!(saves.views().get(&key()), None);
    }

    #[test]
    fn a_removed_accounts_saves_stop_and_are_forgotten() {
        let mut saves = Saves::default();
        let stopped = Rc::new(Cell::new(false));
        let flag = Rc::clone(&stopped);
        saves.started(key(), move || flag.set(true), false);
        saves.keep_accounts(|_| false);
        assert!(stopped.get());
        assert!(!saves.is_saving(key()) && saves.views().is_empty());
    }
}
