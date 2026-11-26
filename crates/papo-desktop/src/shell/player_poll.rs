//! The window's clock for provider players (`docs/design.md` §15.3): a player below half shown
//! for 10 s is released (the window hidden or another conversation shown counts as out of
//! view, the viewer as in view), and a loading player's ring shows after half a second.

use std::collections::HashMap;
use std::time::Duration;

use rune::Tx;

use super::Shell;
use crate::design::PLAYER_RING_AFTER_S;
use crate::provider_players::{Phase, PlayerKey};

impl Shell {
    /// The window's clock: the out-of-view rule and the loading ring. Returns whether views
    /// change, and asks for the wake each needs.
    pub(super) fn poll_players(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        if self.players.store.live_pages().is_empty() {
            self.players.next_release_s = None;
            return false;
        }
        let span = self
            .tabs
            .shown()
            .and_then(|open| open.tracker.shown_span_pt())
            .filter(|_| self.window_shown);
        let shares: HashMap<PlayerKey, f32> = match span {
            None => HashMap::new(),
            Some((top_pt, bottom_pt)) => self
                .players
                .rects
                .iter()
                .filter(|(key, _)| self.shows_conversation_of(**key))
                .map(|(key, rect)| {
                    (
                        *key,
                        super::received_media::shown_share(*rect, top_pt, bottom_pt),
                    )
                })
                .collect(),
        };
        self.players.store.shown(&shares, clock_s);
        let (commands, next_s) = self.players.store.release_due(clock_s);
        let released = !commands.is_empty();
        self.run_player_commands(commands);
        if let Some(next_s) = next_s
            && self.players.next_release_s != Some(next_s)
        {
            self.players.next_release_s = Some(next_s);
            tx.wake_after(Duration::from_secs_f64((next_s - clock_s).max(0.0)));
        }
        released | self.poll_rings(clock_s, tx)
    }

    /// Loading players whose half-second has passed show their ring.
    fn poll_rings(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        let mut changed = false;
        for (key, _) in self.players.store.live_pages() {
            let Some(Phase::Loading { since_s }) = self.players.store.phase(key) else {
                continue;
            };
            let due_s = since_s + PLAYER_RING_AFTER_S;
            if due_s <= clock_s {
                changed |= self.players.ringing.insert(key);
            } else {
                tx.wake_after(Duration::from_secs_f64(due_s - clock_s));
            }
        }
        changed
    }

    /// Whether `key`'s conversation is the one shown.
    fn shows_conversation_of(&self, key: PlayerKey) -> bool {
        self.tabs
            .shown()
            .is_some_and(|open| open.key.account == key.account && open.key.channel == key.channel)
    }
}
