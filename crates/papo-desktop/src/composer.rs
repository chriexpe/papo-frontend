//! The composer's state: each channel's edit buffer and where its draft
//! stands against the SDK's, what a send takes, and when typing is reported.
//!
//! The draft is the SDK's (`OutgoingService::draft` / `save_draft`); a
//! buffer here is the field's text while it is edited, loaded from the SDK
//! when its channel first shows and saved back after a pause in typing, on a
//! switch, on hide and at exit. Every change returns the [`Effect`]s the
//! shell carries out (`shell/composer.rs`); nothing here talks to the SDK.
//! The pill is `view`'s, the Enter setting `prefs`'.

pub(crate) mod prefs;
#[cfg(test)]
mod tests;
mod view;

use std::collections::HashMap;

use papo_sdk::{AccountId, Uuid};
use rune::TextEdit;

#[cfg(test)]
pub(crate) use self::view::COMPOSER_KEY;
pub(crate) use self::view::pill;
use crate::timeline::TimelineKey;

/// How long typing must pause before the draft is saved, in seconds.
const SAVE_AFTER_S: f64 = 1.0;

/// How often typing is reported while it goes on, in seconds. Others see it
/// for ten seconds after each report.
const TYPING_EVERY_S: f64 = 5.0;

/// The longest message the server takes, in characters (Unicode scalar
/// values, as the SDK counts them).
pub(crate) const MAX_MESSAGE_CHARS: usize = 8192;

/// What the shell does for the composer.
#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    /// Read the channel's draft from the SDK.
    Load(TimelineKey),
    /// Save `text` as the channel's draft. `seq` orders writes: one older
    /// than the last written for the channel is dropped.
    Save {
        key: TimelineKey,
        text: String,
        seq: u64,
    },
    /// Queue `text` as a new message with local identity `local`, then
    /// clear the stored draft unless a newer save (`seq`) landed first.
    Send {
        key: TimelineKey,
        text: String,
        local: Uuid,
        seq: u64,
    },
    /// Tell the channel the user is typing.
    ReportTyping(TimelineKey),
    /// Call the composer's poll again in this many seconds.
    WakeAfter(f64),
}

/// One channel's buffer.
pub(crate) struct Draft {
    pub(crate) edit: TextEdit,
    state: DraftState,
}

/// Where a buffer stands against the SDK's stored draft.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DraftState {
    /// The stored draft is being read; the field takes no typing until it
    /// arrives, so typing never races the restore.
    Loading,
    /// The stored draft holds what the field shows, or a save of it is on
    /// its way.
    Saved,
    /// The field changed; it is saved at `due_s` on the window's clock.
    Unsaved { due_s: f64 },
}

/// What pressing Send (or Enter) did.
#[derive(Debug, PartialEq)]
pub(crate) enum Sent {
    /// Nothing to send: the field is empty, blank, or still loading.
    Nothing,
    /// Over [`MAX_MESSAGE_CHARS`]: the field keeps the text.
    TooLong,
    /// The text left the field; the effect queues it.
    Queued(Effect),
}

/// Every channel's buffer and the composer shown.
#[derive(Default)]
pub(crate) struct Composer {
    drafts: HashMap<TimelineKey, Draft>,
    shown: Option<TimelineKey>,
    /// The last write order handed out.
    seq: u64,
    /// When the wake asked for arrives; one at a time.
    wake_s: Option<f64>,
    /// The channel typing was last reported in, and when.
    typing_reported: Option<(TimelineKey, f64)>,
    /// A channel whose last send failed: its text is back in the field and
    /// a line says so until the field changes.
    failed: Option<TimelineKey>,
    /// What [`Composer::field_mut`] edits when no field takes typing.
    spare: TextEdit,
    /// The field takes the keyboard once one shows (the app starts with
    /// it there).
    focus_pending: bool,
}

impl Composer {
    /// The shown channel's buffer, when it has one.
    pub(crate) fn shown(&self) -> Option<(TimelineKey, &Draft)> {
        let key = self.shown?;
        self.drafts.get(&key).map(|draft| (key, draft))
    }

    /// The shown channel's field, for the editor's `on_edit`. An event
    /// that arrives once no field takes typing (the channel left, or its
    /// draft loads) edits a spare buffer that nothing reads.
    pub(crate) fn field_mut(&mut self) -> &mut TextEdit {
        let Composer {
            drafts,
            shown,
            spare,
            ..
        } = self;
        match shown.and_then(|key| drafts.get_mut(&key)) {
            Some(draft) if draft.state != DraftState::Loading => &mut draft.edit,
            _ => {
                *spare = TextEdit::new("");
                spare
            }
        }
    }

    /// Whether the shown channel's draft is still being read.
    pub(crate) fn is_loading(&self) -> bool {
        self.shown()
            .is_some_and(|(_, draft)| draft.state == DraftState::Loading)
    }

    /// Whether the shown channel's last send failed and its text came back.
    pub(crate) fn send_failed_here(&self) -> bool {
        self.shown.is_some() && self.failed == self.shown
    }

    /// How many characters the shown field holds past the limit; zero
    /// within it.
    pub(crate) fn chars_over(&self) -> usize {
        self.shown().map_or(0, |(_, draft)| {
            draft
                .edit
                .text
                .chars()
                .count()
                .saturating_sub(MAX_MESSAGE_CHARS)
        })
    }

    /// Whether the shown field holds anything Send would send.
    pub(crate) fn can_send(&self) -> bool {
        self.shown().is_some_and(|(_, draft)| {
            draft.state != DraftState::Loading && !draft.edit.text.trim().is_empty()
        }) && self.chars_over() == 0
    }

    /// Shows the composer of `key` (none: no text channel shows). The
    /// channel left saves what it has not saved; one shown for the first
    /// time loads its draft. The field keeps the keyboard if it had it.
    pub(crate) fn show(&mut self, key: Option<TimelineKey>) -> Vec<Effect> {
        if key == self.shown {
            return Vec::new();
        }
        let mut effects = Vec::new();
        let mut focused = false;
        if let Some(left) = self.shown.take()
            && let Some(draft) = self.drafts.get_mut(&left)
        {
            focused = std::mem::take(&mut draft.edit.focused);
            effects.extend(self.save(left));
        }
        self.shown = key;
        if let Some(key) = key {
            let draft = self.drafts.entry(key).or_insert_with(|| {
                effects.push(Effect::Load(key));
                Draft {
                    edit: TextEdit::new(""),
                    state: DraftState::Loading,
                }
            });
            draft.edit.focused |= focused || std::mem::take(&mut self.focus_pending);
        }
        effects
    }

    /// Gives the shown field the keyboard, or the next one shown when none
    /// shows yet (the app starts with it there).
    pub(crate) fn focus(&mut self) {
        match self.shown.and_then(|key| self.drafts.get_mut(&key)) {
            Some(draft) => draft.edit.focused = true,
            None => self.focus_pending = true,
        }
    }

    /// The SDK read `key`'s stored draft: `text`, empty when there is
    /// none. Only a buffer still loading takes it; the caret goes to its
    /// end. Reading it again later never replaces what was typed since.
    pub(crate) fn loaded(&mut self, key: TimelineKey, text: String) -> bool {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return false;
        };
        if draft.state != DraftState::Loading {
            return false;
        }
        let end = text.len();
        draft.edit.replace_text(text);
        draft.edit.set_selection(end, end);
        draft.state = DraftState::Saved;
        true
    }

    /// The shown field's text changed at `clock_s`: the draft is saved once
    /// typing pauses, and typing is reported at most every
    /// [`TYPING_EVERY_S`].
    pub(crate) fn edited(&mut self, clock_s: f64) -> Vec<Effect> {
        let Some(key) = self.shown else {
            return Vec::new();
        };
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Vec::new();
        };
        if draft.state == DraftState::Loading {
            return Vec::new();
        }
        let typing = !draft.edit.text.trim().is_empty();
        let due_s = clock_s + SAVE_AFTER_S;
        draft.state = DraftState::Unsaved { due_s };
        if self.failed == Some(key) {
            self.failed = None;
        }
        let mut effects = Vec::new();
        if self.wake_s.is_none() {
            self.wake_s = Some(due_s);
            effects.push(Effect::WakeAfter(SAVE_AFTER_S));
        }
        let reported_lately = self
            .typing_reported
            .is_some_and(|(at, at_s)| at == key && clock_s - at_s < TYPING_EVERY_S);
        if typing && !reported_lately {
            self.typing_reported = Some((key, clock_s));
            effects.push(Effect::ReportTyping(key));
        }
        effects
    }

    /// The window's poll at `clock_s`: saves the drafts whose pause is
    /// over, and asks for the next wake when one still waits.
    pub(crate) fn poll(&mut self, clock_s: f64) -> Vec<Effect> {
        if self.wake_s.is_none_or(|wake_s| wake_s > clock_s) {
            return Vec::new();
        }
        self.wake_s = None;
        let due: Vec<TimelineKey> = self
            .drafts
            .iter()
            .filter(|(_, draft)| {
                matches!(draft.state, DraftState::Unsaved { due_s } if due_s <= clock_s)
            })
            .map(|(key, _)| *key)
            .collect();
        let mut effects: Vec<Effect> = due.into_iter().filter_map(|key| self.save(key)).collect();
        let next_s = self
            .drafts
            .values()
            .filter_map(|draft| match draft.state {
                DraftState::Unsaved { due_s } => Some(due_s),
                DraftState::Loading | DraftState::Saved => None,
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
            .filter(|(_, draft)| matches!(draft.state, DraftState::Unsaved { .. }))
            .map(|(key, _)| *key)
            .collect();
        unsaved
            .into_iter()
            .filter_map(|key| self.save(key))
            .collect()
    }

    /// Send was pressed: the shown field's text leaves it as a new message
    /// with identity `local`. The stored draft keeps the text until the SDK
    /// has queued it (`Effect::Send`).
    pub(crate) fn send(&mut self, local: Uuid) -> Sent {
        if !self.can_send() {
            return if self.chars_over() > 0 {
                Sent::TooLong
            } else {
                Sent::Nothing
            };
        }
        let Some(key) = self.shown else {
            return Sent::Nothing;
        };
        let seq = self.next_seq();
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Sent::Nothing;
        };
        let text = std::mem::take(&mut draft.edit.text);
        // A sent message never comes back through undo.
        draft.edit.replace_text("");
        draft.state = DraftState::Saved;
        if self.failed == Some(key) {
            self.failed = None;
        }
        Sent::Queued(Effect::Send {
            key,
            text,
            local,
            seq,
        })
    }

    /// The SDK did not queue `text` for `key`: it goes back in the field,
    /// before whatever was typed since, and is saved again.
    pub(crate) fn send_failed(&mut self, key: TimelineKey, text: String) -> Vec<Effect> {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Vec::new();
        };
        let restored = if draft.edit.text.is_empty() {
            text
        } else {
            format!("{text}\n{}", draft.edit.text)
        };
        let end = restored.len();
        draft.edit.replace_text(restored);
        draft.edit.set_selection(end, end);
        draft.state = DraftState::Unsaved { due_s: 0.0 };
        self.failed = Some(key);
        self.save(key).into_iter().collect()
    }

    /// Forgets the buffers of accounts no longer saved; their drafts went
    /// with the account.
    pub(crate) fn retain_accounts(&mut self, keep: impl Fn(AccountId) -> bool) {
        self.drafts.retain(|key, _| keep(key.account));
        if self.shown.is_some_and(|key| !keep(key.account)) {
            self.shown = None;
        }
    }

    /// The save of `key`'s buffer, when it has changes; it counts as saved
    /// from now on.
    fn save(&mut self, key: TimelineKey) -> Option<Effect> {
        let seq = self.next_seq();
        let draft = self.drafts.get_mut(&key)?;
        if !matches!(draft.state, DraftState::Unsaved { .. }) {
            return None;
        }
        draft.state = DraftState::Saved;
        Some(Effect::Save {
            key,
            text: draft.edit.text.clone(),
            seq,
        })
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}
