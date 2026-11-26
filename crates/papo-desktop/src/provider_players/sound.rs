//! One sound at a time among provider players (`docs/design.md` §15.3): a player that starts
//! sounding quiets the others, other media quiet them all, and a quieted player sounds again
//! when it is used. Quieting mutes the page: Papo can't pause it without injecting script. The
//! rest of the store's rules are `provider_players`'.

use super::{Command, PageId, PlayerKey, PlayerStore};

/// What a page's sound change means for the rest of the app.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SoundChange {
    /// A player started sounding: other media go quiet.
    pub(crate) quiet_others: bool,
    pub(crate) commands: Vec<Command>,
}

impl PlayerStore {
    /// Page `page` started or stopped sounding.
    pub(crate) fn sound(&mut self, page: PageId, sounding: bool) -> SoundChange {
        let Some(key) = self.key_of(page) else {
            return SoundChange::default();
        };
        let muted = self.entries.get(&key).is_some_and(|entry| entry.muted);
        if !sounding || muted {
            return SoundChange::default();
        }
        SoundChange {
            quiet_others: true,
            commands: self.quiet(Some(key)),
        }
    }

    /// Other media started sounding: every player goes quiet.
    pub(crate) fn quiet_all(&mut self) -> Vec<Command> {
        self.quiet(None)
    }

    /// `key` was used: a quieted player sounds again, and the others go quiet. Returns the
    /// commands and whether other media must go quiet.
    pub(crate) fn unquiet(&mut self, key: PlayerKey) -> SoundChange {
        self.touch(key);
        let Some(entry) = self.entries.get_mut(&key) else {
            return SoundChange::default();
        };
        let Some(page) = entry.page.filter(|_| entry.muted) else {
            return SoundChange::default();
        };
        entry.muted = false;
        let mut commands = vec![Command::Mute(page, false)];
        commands.extend(self.quiet(Some(key)));
        SoundChange {
            quiet_others: true,
            commands,
        }
    }

    /// Mutes every live player but `except`.
    fn quiet(&mut self, except: Option<PlayerKey>) -> Vec<Command> {
        let mut commands = Vec::new();
        for (key, entry) in &mut self.entries {
            if Some(*key) == except || entry.muted {
                continue;
            }
            if let Some(page) = entry.page {
                entry.muted = true;
                commands.push(Command::Mute(page, true));
            }
        }
        commands
    }
}
