//! The composer's state: each channel's edit buffer and where its draft
//! stands against the SDK's, what a send takes, and when typing is reported.
//!
//! The draft is the SDK's (`OutgoingService::draft` / `save_draft`); a
//! buffer here is the field's text while it is edited, loaded from the SDK
//! when its channel first shows and saved back after a pause in typing, on a
//! switch, on hide and at exit. Every change returns the [`Effect`]s the
//! shell carries out (`shell/composer.rs`); nothing here talks to the SDK.
//! The pill is `view`'s, the Enter setting `prefs`'. Markdown, pairs and
//! mentions in the field are `markdown`'s, `pairs`' and `mentions`'; the
//! keys and the @ picker that use them are `editing`'s and `picker`'s.
//! Replying and editing (the mode, its bars and its moves) are `mode`'s,
//! `bars`' and `replying`'s.

pub(crate) mod bars;
mod draft;
mod editing;
mod effect;
mod markdown;
pub(crate) mod mentions;
pub(crate) mod mode;
mod pairs;
pub(crate) mod picker;
pub(crate) mod prefs;
mod replying;
mod saving;
#[cfg(test)]
mod tests;
pub(crate) mod view;

use std::collections::HashMap;

use papo_sdk::{AccountId, Uuid};
use papo_sdk_ui::message_content::DecodedComposerMentions;
use rune::TextEdit;

pub(crate) use self::draft::Draft;
use self::draft::DraftState;
pub(crate) use self::effect::{Effect, Sent};
use self::mode::{Mode, Reply};
pub(crate) use self::pairs::{Keystroke, wrap_format};
pub(crate) use self::picker::Picker;
pub(crate) use self::view::{COMPOSER_KEY, COMPOSER_PILL_KEY, PillContext, pill};
use crate::timeline::TimelineKey;

/// How long typing must pause before the draft is saved, in seconds.
const SAVE_AFTER_S: f64 = 1.0;

/// How often typing is reported while it goes on, in seconds. Others see it
/// for ten seconds after each report.
const TYPING_EVERY_S: f64 = 5.0;

/// How early a wake may arrive and still count as due, in seconds: a
/// timer that fires a hair before its deadline must not stall the save.
const WAKE_SLACK_S: f64 = 0.001;

/// The longest message the server takes, in characters (Unicode scalar
/// values, as the SDK counts them).
pub(crate) const MAX_MESSAGE_CHARS: usize = 8192;

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
    /// The typing key that last reached the field, and when on the
    /// window's clock: its own change in that same event may close a pair,
    /// and no other change does.
    typed: Option<(f64, Keystroke)>,
    /// The @ picker, while an `@` is being typed in the shown field.
    picker: Option<Picker>,
    /// The `@` whose picker Escape closed, by its offset in the shown
    /// field: it stays closed until another `@` is typed.
    dismissed_at: Option<(TimelineKey, usize)>,
}

impl Composer {
    /// The shown channel's buffer, when it has one.
    pub(crate) fn shown(&self) -> Option<(TimelineKey, &Draft)> {
        let key = self.shown?;
        self.drafts.get(&key).map(|draft| (key, draft))
    }

    /// The shown channel's field, for the editor's `on_edit`. An event
    /// that arrives while no field takes typing (the channel left, or its
    /// draft still loads) edits a spare buffer that nothing reads. Rune
    /// calls this several times per event, and each call empties the
    /// spare; its text reads the same before and after, so no change
    /// handler runs for it.
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
            draft.wire_chars.saturating_sub(MAX_MESSAGE_CHARS)
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
        self.picker = None;
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
                Draft::loading()
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

    /// The SDK read `key`'s stored draft: `decoded`, empty when there is
    /// none, its mentions named. Only a buffer still loading takes it; the
    /// caret goes to its end. Reading it again later never replaces what
    /// was typed since.
    pub(crate) fn loaded(
        &mut self,
        key: TimelineKey,
        decoded: DecodedComposerMentions,
        reply_to: Option<Uuid>,
        mention: bool,
    ) -> bool {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return false;
        };
        if draft.state != DraftState::Loading {
            return false;
        }
        mentions::restore(&mut draft.edit, decoded);
        draft.replaced();
        draft.state = DraftState::Saved;
        draft.mode = Mode::Writing {
            reply: reply_to.map(|id| Reply {
                id,
                original: None,
                mention,
            }),
        };
        true
    }

    /// The SDK could not read `key`'s stored draft (signed out, storage).
    /// The buffer is forgotten rather than taken as empty, which a save
    /// would then write over the stored draft; it is read again the next
    /// time the channel shows.
    pub(crate) fn load_failed(&mut self, key: TimelineKey) -> bool {
        let Some(draft) = self.drafts.get(&key) else {
            return false;
        };
        if draft.state != DraftState::Loading {
            return false;
        }
        let focused = draft.edit.focused;
        self.drafts.remove(&key);
        if self.shown == Some(key) {
            self.shown = None;
            self.focus_pending |= focused;
        }
        true
    }

    /// The save with order `seq` of `key`'s buffer failed: unless the
    /// buffer was saved again since, it counts as unsaved.
    pub(crate) fn save_failed(&mut self, key: TimelineKey, seq: u64) -> bool {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return false;
        };
        if draft.state != DraftState::Saved || draft.last_save != Some(seq) {
            return false;
        }
        draft.state = DraftState::SaveFailed;
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
        // The same event's clock, to the bit: a key's mark is for its own
        // change only.
        let keystroke = self
            .typed
            .take()
            .filter(|(at_s, _)| at_s.to_bits() == clock_s.to_bits());
        let keystroke = keystroke.as_ref().map(|(_, keystroke)| keystroke);
        draft.pairs.changed(&mut draft.edit, keystroke);
        draft.refresh();
        if draft.mode.editing().is_some() {
            // An edit is neither saved nor reported as typing.
            return Vec::new();
        }
        let typing = !draft.edit.text.trim().is_empty();
        let due_s = clock_s + SAVE_AFTER_S;
        draft.state = DraftState::Unsaved { due_s };
        if self.failed == Some(key) {
            self.failed = None;
        }
        let mut effects = Vec::new();
        // A wake already past is one the poll missed: ask again.
        if self.wake_s.is_none_or(|wake_s| wake_s < clock_s) {
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

    /// Send was pressed: the shown field's text leaves it as a new message
    /// with identity `local`. The stored draft keeps the text until the SDK
    /// has queued it (`Effect::Send`).
    pub(crate) fn send(&mut self, local: Uuid) -> Sent {
        if self
            .shown()
            .is_some_and(|(_, draft)| draft.mode.editing().is_some())
        {
            return self.save_edit(local);
        }
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
        let text = mentions::wire(&draft.edit);
        let reply_to = draft.mode.reply().map(|reply| reply.id);
        draft.mode = Mode::default();
        // A sent message never comes back through undo.
        draft.edit.replace_text("");
        draft.replaced();
        draft.state = DraftState::Saved;
        draft.last_save = Some(seq);
        if self.failed == Some(key) {
            self.failed = None;
        }
        self.picker = None;
        Sent::Queued(Effect::Send {
            key,
            text,
            reply_to,
            local,
            seq,
        })
    }

    /// The SDK did not queue `decoded` for `key`: it goes back in the
    /// field, before whatever was typed since, and is saved again.
    pub(crate) fn send_failed(
        &mut self,
        key: TimelineKey,
        decoded: DecodedComposerMentions,
        reply_to: Option<Uuid>,
        mention: bool,
    ) -> Vec<Effect> {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Vec::new();
        };
        // During an edit, the text and its reply go to the draft the edit
        // set aside, and come back with it.
        if let Mode::Editing(editing) = &mut draft.mode {
            let before = &mut editing.before;
            mentions::prepend(&mut before.edit, decoded);
            if let (Some(id), None) = (reply_to, &before.reply) {
                before.reply = Some(Reply {
                    id,
                    original: None,
                    mention,
                });
            }
            before.send_failed = true;
            return Vec::new();
        }
        // The reply comes back with its text, unless another one began.
        if let (
            Some(id),
            Mode::Writing {
                reply: reply @ None,
            },
        ) = (reply_to, &mut draft.mode)
        {
            *reply = Some(Reply {
                id,
                original: None,
                mention,
            });
        }
        mentions::prepend(&mut draft.edit, decoded);
        draft.replaced();
        // Its query, if open, no longer points at the same text.
        self.picker = None;
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
        if self.failed.is_some_and(|key| !keep(key.account)) {
            self.failed = None;
        }
        if self
            .typing_reported
            .is_some_and(|(key, _)| !keep(key.account))
        {
            self.typing_reported = None;
        }
    }
}
