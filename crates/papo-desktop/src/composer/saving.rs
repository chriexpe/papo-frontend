//! When buffers are saved: after a pause in typing (the window's poll), all
//! at once on hide and exit (flush), and what one save writes: the wire
//! form with its mentions, ordered by `seq`.

use super::{Composer, DraftState, Effect, TimelineKey, WAKE_SLACK_S, mentions};

impl Composer {
    /// The window's poll at `clock_s`: saves the drafts whose pause is
    /// over, and asks for the next wake when one still waits.
    pub(crate) fn poll(&mut self, clock_s: f64) -> Vec<Effect> {
        let now_s = clock_s + WAKE_SLACK_S;
        if self.wake_s.is_none_or(|wake_s| wake_s > now_s) {
            return Vec::new();
        }
        self.wake_s = None;
        let due: Vec<TimelineKey> = self
            .drafts
            .iter()
            .filter(
                |(_, draft)| matches!(draft.state, DraftState::Unsaved { due_s } if due_s <= now_s),
            )
            .map(|(key, _)| *key)
            .collect();
        let mut effects: Vec<Effect> = due.into_iter().filter_map(|key| self.save(key)).collect();
        let next_s = self
            .drafts
            .values()
            .filter_map(|draft| match draft.state {
                DraftState::Unsaved { due_s } => Some(due_s),
                DraftState::Loading | DraftState::Saved | DraftState::SaveFailed => None,
            })
            .reduce(f64::min);
        if let Some(next_s) = next_s {
            self.wake_s = Some(next_s);
            effects.push(Effect::WakeAfter(next_s - clock_s));
        }
        effects
    }

    /// Saves every draft not yet saved: the window hides or closes.
    pub(crate) fn flush(&mut self) -> Vec<Effect> {
        let unsaved: Vec<TimelineKey> = self
            .drafts
            .iter()
            .filter(|(_, draft)| draft.state.unsaved())
            .map(|(key, _)| *key)
            .collect();
        unsaved
            .into_iter()
            .filter_map(|key| self.save(key))
            .collect()
    }

    /// The save of `key`'s buffer, when it has changes; it counts as saved
    /// from now on.
    /// An edit is never saved: the stored draft keeps the field as it was
    /// before the edit.
    pub(super) fn save(&mut self, key: TimelineKey) -> Option<Effect> {
        let draft = self.drafts.get(&key)?;
        if !draft.state.unsaved() || draft.mode.editing().is_some() {
            return None;
        }
        let seq = self.next_seq();
        let draft = self.drafts.get_mut(&key)?;
        draft.state = DraftState::Saved;
        draft.last_save = Some(seq);
        Some(Effect::Save {
            key,
            text: mentions::wire(&draft.edit),
            mentions: mentions::bindings(&draft.edit)
                .into_iter()
                .map(|binding| binding.id)
                .collect(),
            reply_to: draft.mode.reply().map(|reply| reply.id),
            seq,
        })
    }

    pub(super) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}
