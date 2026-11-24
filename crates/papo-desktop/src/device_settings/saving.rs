//! The order of one owner's writes (`docs/design.md` §8, Saving): one at a
//! time, the latest change waiting behind the running one, and a setting
//! that shows what the file holds once its writes are done.
//!
//! Owns the queue ([`Saving`]), a setting kept with it ([`Stored`]) and a
//! remembered key that keeps its value for the run when a write fails
//! ([`Remembered`]); the file is `device_settings`'s, the values their
//! owner's.

/// One owner's writes: none running, or one running with the latest change
/// since waiting for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Saving<T> {
    #[default]
    Idle,
    Writing {
        next: Option<T>,
    },
}

/// What happens to the writes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SavingEvent<T> {
    /// A change to write.
    Changed(T),
    /// The running write landed, or did not.
    Written { ok: bool },
}

impl<T> Saving<T> {
    /// The writes after `event`, and what to write now.
    pub(crate) fn step(self, event: SavingEvent<T>) -> (Saving<T>, Option<T>) {
        match (self, event) {
            (Saving::Idle, SavingEvent::Changed(value)) => {
                (Saving::Writing { next: None }, Some(value))
            }
            // The latest change waits; one before it that never ran is
            // replaced, as the file only needs the last.
            (Saving::Writing { .. }, SavingEvent::Changed(value)) => {
                (Saving::Writing { next: Some(value) }, None)
            }
            (Saving::Writing { next: Some(next) }, SavingEvent::Written { ok: true }) => {
                (Saving::Writing { next: None }, Some(next))
            }
            // Done, or failed: a failure drops the change waiting behind it,
            // whose write would fail too.
            (Saving::Writing { .. } | Saving::Idle, SavingEvent::Written { .. }) => {
                (Saving::Idle, None)
            }
        }
    }
}

/// A setting and its writes: what shows (the saved value, or a change on
/// its way to the file) and what the file holds. A change shows at once; a
/// failed write takes it back to what the file holds.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stored<T> {
    shown: T,
    saved: T,
    saving: Saving<T>,
}

impl<T: Clone + PartialEq> Stored<T> {
    /// `value`, as the file holds it.
    pub(crate) fn new(value: T) -> Stored<T> {
        Stored {
            shown: value.clone(),
            saved: value,
            saving: Saving::Idle,
        }
    }

    /// What shows.
    pub(crate) fn shown(&self) -> &T {
        &self.shown
    }

    /// Shows `value` at once; the value to write now, unless a write is
    /// running (it then follows that one) or nothing changed.
    pub(crate) fn change(&mut self, value: T) -> Option<T> {
        if value == self.shown {
            return None;
        }
        self.shown = value.clone();
        self.step(SavingEvent::Changed(value))
    }

    /// The write of `value` landed (`ok`) or did not, which takes what
    /// shows back to what the file holds; the value to write next, if any.
    pub(crate) fn written(&mut self, value: T, ok: bool) -> Option<T> {
        if ok {
            self.saved = value;
        } else {
            self.shown = self.saved.clone();
        }
        self.step(SavingEvent::Written { ok })
    }

    fn step(&mut self, event: SavingEvent<T>) -> Option<T> {
        let (saving, write) = std::mem::take(&mut self.saving).step(event);
        self.saving = saving;
        write
    }
}

/// One key remembered across launches (the layout, the recent jumps):
/// written in order like a setting, but a failed write only costs the next
/// launch the value, so this run keeps it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Remembered {
    key: &'static str,
    file: super::SettingsFile,
    saving: Saving<serde_json::Value>,
}

impl Remembered {
    pub(crate) fn new(key: &'static str, file: super::SettingsFile) -> Remembered {
        Remembered {
            key,
            file,
            saving: Saving::Idle,
        }
    }

    /// `value` is the key's now; the write to run now, unless one is
    /// running (it then follows that one).
    pub(crate) fn remember(&mut self, value: serde_json::Value) -> Option<super::Write> {
        self.step(SavingEvent::Changed(value))
    }

    /// The running write landed, or did not (logged); the write to run
    /// next, if any.
    pub(crate) fn written(&mut self, ok: bool) -> Option<super::Write> {
        if !ok {
            log::warn!("{} was not saved; it holds for this run", self.key);
        }
        self.step(SavingEvent::Written { ok })
    }

    fn step(&mut self, event: SavingEvent<serde_json::Value>) -> Option<super::Write> {
        let (saving, value) = std::mem::take(&mut self.saving).step(event);
        self.saving = saving;
        value.map(|value| self.file.write(vec![(self.key, value)]))
    }
}

#[cfg(test)]
mod tests {
    use super::Stored;

    #[test]
    fn a_change_shows_at_once_and_is_written() {
        let mut stored = Stored::new(1);
        assert_eq!(stored.change(2), Some(2));
        assert_eq!(*stored.shown(), 2);
        assert_eq!(stored.written(2, true), None);
        assert_eq!(*stored.shown(), 2);
    }

    #[test]
    fn changes_during_a_write_wait_and_only_the_last_is_written() {
        let mut stored = Stored::new(1);
        assert_eq!(stored.change(2), Some(2));
        assert_eq!(stored.change(3), None);
        assert_eq!(stored.change(4), None);
        assert_eq!(stored.written(2, true), Some(4));
        assert_eq!(stored.written(4, true), None);
        assert_eq!(*stored.shown(), 4);
    }

    #[test]
    fn a_failed_write_goes_back_to_the_file_and_drops_what_waited() {
        let mut stored = Stored::new(1);
        assert_eq!(stored.change(2), Some(2));
        assert_eq!(stored.change(3), None);
        assert_eq!(stored.written(2, false), None);
        assert_eq!(*stored.shown(), 1);
        // The queue is free again.
        assert_eq!(stored.change(5), Some(5));
    }

    #[test]
    fn no_change_writes_nothing() {
        let mut stored = Stored::new(1);
        assert_eq!(stored.change(1), None);
    }
}
