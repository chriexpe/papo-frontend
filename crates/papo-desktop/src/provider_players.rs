//! Provider players' rules (`docs/design.md` §15.3, §15.5, §15.6): a poster until you press
//! play, the ask inside the card for a site you haven't allowed, loading, playing, released
//! after 10 s below half shown (the poster returns with its note), failed. At most two live
//! players: starting a third releases the one touched longest ago. One sound at a time: a
//! player that starts sounding quiets the others, other media quiets them all, and a quieted
//! player sounds again when you use it. The viewer takes a live player over, which keeps it
//! live wherever its row is.
//!
//! Pure: it decides, and says what the WebViews must do ([`Command`]); the shell carries the
//! commands out, draws, and asks the person (`web_view`, `link_cards`). A player's key is its
//! message's embed, since the same video in two messages is two players.

use std::collections::HashMap;

use papo_sdk::{AccountId, Uuid};
use papo_sdk_ui::ProviderPlayer;

use crate::web_view::PageId;

/// The most live players.
pub(crate) const MAX_LIVE: usize = 2;
/// How long a player below half shown stays live.
pub(crate) const RELEASE_AFTER_S: f64 = 10.0;
/// The share of its box a player shows to count as in view.
pub(crate) const SHOWN_SHARE: f32 = 0.5;

/// Which provider video: an embed of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PlayerKey {
    pub(crate) account: AccountId,
    pub(crate) channel: Uuid,
    pub(crate) message: Uuid,
    pub(crate) embed: Uuid,
}

/// Where a player is; no entry is the poster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Phase {
    /// The ask is open on the dimmed poster.
    Asking,
    /// Its page is loading since `since_s` (the window's clock); the ring shows after 0.5 s.
    Loading { since_s: f64 },
    /// Its page draws.
    Playing,
    /// Released after it left the view: the poster with its note.
    Stopped,
    /// Its page couldn't load.
    Failed,
}

impl Phase {
    /// Whether it has a page.
    pub(crate) fn is_live(self) -> bool {
        matches!(self, Phase::Loading { .. } | Phase::Playing)
    }
}

/// What the WebViews must do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    /// Open `page` for `key`'s player.
    Open {
        key: PlayerKey,
        page: PageId,
        player: ProviderPlayer,
        muted: bool,
    },
    Close(PageId),
    Mute(PageId, bool),
}

struct Entry {
    phase: Phase,
    player: ProviderPlayer,
    page: Option<PageId>,
    /// When it was last used, for which goes first.
    touched: u64,
    /// Since when it has been below half shown (the window's clock).
    out_since_s: Option<f64>,
    muted: bool,
}

/// What a page's sound change means for the rest of the app.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct SoundChange {
    /// A player started sounding: other media go quiet.
    pub(crate) quiet_others: bool,
    pub(crate) commands: Vec<Command>,
}

/// The players.
#[derive(Default)]
pub(crate) struct PlayerStore {
    entries: HashMap<PlayerKey, Entry>,
    next_page: PageId,
    clock: u64,
    viewer: Option<PlayerKey>,
}

impl PlayerStore {
    /// `key`'s phase; `None` is the poster.
    pub(crate) fn phase(&self, key: PlayerKey) -> Option<Phase> {
        self.entries.get(&key).map(|entry| entry.phase)
    }

    /// `key`'s page, while it is live.
    pub(crate) fn page(&self, key: PlayerKey) -> Option<PageId> {
        self.entries.get(&key).and_then(|entry| entry.page)
    }

    /// The player with page `page`.
    pub(crate) fn key_of(&self, page: PageId) -> Option<PlayerKey> {
        self.entries
            .iter()
            .find(|(_, entry)| entry.page == Some(page))
            .map(|(key, _)| *key)
    }

    /// Whose player `key` is.
    pub(crate) fn provider(&self, key: PlayerKey) -> Option<papo_sdk_ui::PlayerProvider> {
        self.entries.get(&key).map(|entry| entry.player.provider)
    }

    /// Every player that isn't a poster.
    pub(crate) fn keys(&self) -> impl Iterator<Item = PlayerKey> + '_ {
        self.entries.keys().copied()
    }

    /// The live players' pages.
    pub(crate) fn live_pages(&self) -> Vec<(PlayerKey, PageId)> {
        self.entries
            .iter()
            .filter_map(|(key, entry)| entry.page.map(|page| (*key, page)))
            .collect()
    }

    /// The player the viewer shows.
    pub(crate) fn viewer(&self) -> Option<PlayerKey> {
        self.viewer
    }

    /// A press on play for a site not allowed: the ask opens on `key`'s poster.
    pub(crate) fn ask(&mut self, key: PlayerKey, player: ProviderPlayer) {
        let touched = self.tick();
        self.entries.insert(
            key,
            Entry {
                phase: Phase::Asking,
                player,
                page: None,
                touched,
                out_since_s: None,
                muted: false,
            },
        );
    }

    /// The ask is cancelled, or a player closed: the poster returns.
    pub(crate) fn close(&mut self, key: PlayerKey) -> Vec<Command> {
        if self.viewer == Some(key) {
            self.viewer = None;
        }
        self.entries
            .remove(&key)
            .and_then(|entry| entry.page)
            .map(Command::Close)
            .into_iter()
            .collect()
    }

    /// Starts `key`'s player at `now_s`, from the beginning; a third live player releases the
    /// one touched longest ago, whose poster returns. Its sound starts on unless `muted`.
    pub(crate) fn start(
        &mut self,
        key: PlayerKey,
        player: ProviderPlayer,
        muted: bool,
        now_s: f64,
    ) -> Vec<Command> {
        let mut commands = self.close(key);
        self.next_page += 1;
        let page = self.next_page;
        let touched = self.tick();
        self.entries.insert(
            key,
            Entry {
                phase: Phase::Loading { since_s: now_s },
                player: player.clone(),
                page: Some(page),
                touched,
                out_since_s: None,
                muted,
            },
        );
        commands.push(Command::Open {
            key,
            page,
            player,
            muted,
        });
        while self.live_count() > MAX_LIVE {
            let oldest = self
                .entries
                .iter()
                .filter(|(other, entry)| **other != key && entry.phase.is_live())
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(other, _)| *other);
            let Some(oldest) = oldest else {
                break;
            };
            commands.extend(self.close(oldest));
        }
        commands
    }

    /// Starts `key` again from where its poster is: after it stopped or failed.
    pub(crate) fn restart(&mut self, key: PlayerKey, muted: bool, now_s: f64) -> Vec<Command> {
        let Some(player) = self.entries.get(&key).map(|entry| entry.player.clone()) else {
            return Vec::new();
        };
        self.start(key, player, muted, now_s)
    }

    /// `key` was used (pressed, typed into, its buttons): it goes last in line for release.
    pub(crate) fn touch(&mut self, key: PlayerKey) {
        let touched = self.tick();
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.touched = touched;
        }
    }

    /// Page `page` drew its first frame. Returns whether that changed anything.
    pub(crate) fn painted(&mut self, page: PageId) -> bool {
        match self.entry_of(page) {
            Some(entry) if matches!(entry.phase, Phase::Loading { .. }) => {
                entry.phase = Phase::Playing;
                true
            }
            _ => false,
        }
    }

    /// Page `page` failed: its player shows Failed, and its page goes.
    pub(crate) fn failed(&mut self, page: PageId) -> Vec<Command> {
        let Some(key) = self.key_of(page) else {
            return Vec::new();
        };
        if self.viewer == Some(key) {
            self.viewer = None;
        }
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.phase = Phase::Failed;
            entry.page = None;
        }
        vec![Command::Close(page)]
    }

    /// The shares of each live player's box shown now (missing: not drawn), at `now_s`.
    pub(crate) fn shown(&mut self, shares: &HashMap<PlayerKey, f32>, now_s: f64) {
        let viewer = self.viewer;
        for (key, entry) in &mut self.entries {
            if !entry.phase.is_live() {
                continue;
            }
            let in_view =
                Some(*key) == viewer || shares.get(key).is_some_and(|share| *share >= SHOWN_SHARE);
            if in_view {
                entry.out_since_s = None;
            } else if entry.out_since_s.is_none() {
                entry.out_since_s = Some(now_s);
            }
        }
    }

    /// Releases the players out of view for [`RELEASE_AFTER_S`] by `now_s`: their poster
    /// returns with its note. Returns the commands and when the next one is due.
    pub(crate) fn release_due(&mut self, now_s: f64) -> (Vec<Command>, Option<f64>) {
        let mut commands = Vec::new();
        let mut next_s: Option<f64> = None;
        for entry in self.entries.values_mut() {
            let Some(out_s) = entry.out_since_s.filter(|_| entry.phase.is_live()) else {
                continue;
            };
            let due_s = out_s + RELEASE_AFTER_S;
            if due_s <= now_s {
                entry.phase = Phase::Stopped;
                entry.out_since_s = None;
                commands.extend(entry.page.take().map(Command::Close));
            } else {
                next_s = Some(next_s.map_or(due_s, |next| next.min(due_s)));
            }
        }
        (commands, next_s)
    }

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

    /// The viewer takes `key`'s live player (or lets it go with `None`).
    pub(crate) fn set_viewer(&mut self, key: Option<PlayerKey>) {
        self.viewer = key.filter(|key| self.page(*key).is_some());
        if let Some(key) = key {
            self.touch(key);
        }
    }

    /// Every player goes (the players were switched off, or WebViews went away); posters
    /// return.
    pub(crate) fn release_all(&mut self) -> Vec<Command> {
        self.viewer = None;
        self.entries
            .drain()
            .filter_map(|(_, entry)| entry.page)
            .map(Command::Close)
            .collect()
    }

    fn live_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.phase.is_live())
            .count()
    }

    fn entry_of(&mut self, page: PageId) -> Option<&mut Entry> {
        self.entries
            .values_mut()
            .find(|entry| entry.page == Some(page))
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

#[cfg(test)]
mod tests;
