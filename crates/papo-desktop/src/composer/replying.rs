//! Starting and ending a reply or an edit in the shown composer
//! (`docs/design.md` §11). A reply changes the stored draft (its original's
//! ID is part of it), so starting or ending one saves at once. An edit sets
//! the field aside and puts it back when it ends, saved or not.

use papo_sdk::Uuid;
use papo_sdk_ui::ReplyOriginal;
use papo_sdk_ui::message_content::DecodedComposerMentions;

use super::draft::DraftState;
use super::mode::{Before, Editing, Mode, Reply};
use super::{Composer, Effect, MAX_MESSAGE_CHARS, Sent, mentions};
use crate::timeline::TimelineKey;

impl Composer {
    /// Reply to message `id` in the shown channel `key`: the bar opens over
    /// the pill and the field keeps its text. It replaces another reply, and
    /// an edit ends first. `original` is the message as the conversation
    /// shows it; `mention` is where the @ switch starts.
    pub(crate) fn start_reply(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        original: Option<ReplyOriginal>,
        mention: bool,
    ) -> Vec<Effect> {
        if self.shown != Some(key) {
            return Vec::new();
        }
        let mut effects = self.cancel_edit().unwrap_or_default();
        let Some(draft) = self.drafts.get_mut(&key) else {
            return effects;
        };
        if draft.state == DraftState::Loading {
            return effects;
        }
        draft.mode = Mode::Writing {
            reply: Some(Reply {
                id,
                original,
                mention,
            }),
        };
        draft.edit.focused = true;
        effects.extend(self.reply_changed(key));
        effects
    }

    /// Escape or ✕ on the reply bar: the reply ends, the text stays. None
    /// when no reply was open; else the draft's save.
    pub(crate) fn cancel_reply(&mut self) -> Option<Vec<Effect>> {
        let key = self.shown?;
        let draft = self.drafts.get_mut(&key)?;
        let Mode::Writing {
            reply: reply @ Some(_),
        } = &mut draft.mode
        else {
            return None;
        };
        *reply = None;
        Some(self.reply_changed(key))
    }

    /// The @ switch on the reply bar.
    pub(crate) fn toggle_reply_mention(&mut self) {
        let draft = self.shown.and_then(|key| self.drafts.get_mut(&key));
        if let Some(Mode::Writing {
            reply: Some(reply), ..
        }) = draft.map(|draft| &mut draft.mode)
        {
            reply.mention = !reply.mention;
        }
    }

    /// The SDK told what a restored reply's original is. Returns whether
    /// the bar changed.
    pub(crate) fn reply_found(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        original: ReplyOriginal,
    ) -> bool {
        let reply = self
            .drafts
            .get_mut(&key)
            .and_then(|draft| match &mut draft.mode {
                Mode::Writing { reply: Some(reply) } if reply.id == id => Some(reply),
                _ => None,
            });
        match reply {
            Some(reply) => {
                reply.original = Some(original);
                true
            }
            None => false,
        }
    }

    /// The restored replies whose original is not known yet, by channel.
    pub(crate) fn unresolved_replies(&self) -> Vec<(TimelineKey, Uuid)> {
        self.drafts
            .iter()
            .filter_map(|(key, draft)| match draft.mode.reply() {
                Some(reply) if reply.original.is_none() => Some((*key, reply.id)),
                _ => None,
            })
            .collect()
    }

    /// Edit your message `id` in the shown channel `key`, whose text is
    /// `decoded`: the field and any reply are set aside (saved first), the
    /// field takes the message's text with the caret at its end, and Send
    /// becomes Save. Another edit ends first, unsaved.
    pub(crate) fn start_edit(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        decoded: DecodedComposerMentions,
    ) -> Vec<Effect> {
        if self.shown != Some(key) {
            return Vec::new();
        }
        let mut effects = self.cancel_edit().unwrap_or_default();
        effects.extend(self.save(key));
        let Some(draft) = self.drafts.get_mut(&key) else {
            return effects;
        };
        if draft.state == DraftState::Loading {
            return effects;
        }
        let reply = match std::mem::take(&mut draft.mode) {
            Mode::Writing { reply } => reply,
            Mode::Editing(_) => None,
        };
        let before = Before {
            edit: draft.edit.clone(),
            reply,
            send_failed: false,
        };
        mentions::restore(&mut draft.edit, decoded);
        draft.edit.focused = true;
        draft.replaced();
        draft.mode = Mode::Editing(Box::new(Editing { id, before }));
        if self.failed == Some(key) {
            self.failed = None;
        }
        self.picker = None;
        effects
    }

    /// The SDK refused the edit of message `id` in `key`, whose text is
    /// `decoded`: the edit opens again with it (setting aside, and saving,
    /// what the field holds now), and the note says it did not go.
    pub(crate) fn edit_came_back(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        decoded: DecodedComposerMentions,
    ) -> Vec<Effect> {
        if self.shown != Some(key) {
            return Vec::new();
        }
        let effects = self.start_edit(key, id, decoded);
        self.failed = Some(key);
        effects
    }

    /// Escape or ✕ on the edit bar, or the edit saved: the field and reply
    /// set aside come back, and the message is left as it is. None when no
    /// edit was open; else the save of a draft a refused send came back
    /// into meanwhile.
    pub(crate) fn cancel_edit(&mut self) -> Option<Vec<Effect>> {
        let key = self.shown?;
        let draft = self.drafts.get_mut(&key)?;
        draft.mode.editing()?;
        let Mode::Editing(editing) = std::mem::take(&mut draft.mode) else {
            return None;
        };
        let Editing { before, .. } = *editing;
        let focused = draft.edit.focused;
        draft.edit = before.edit;
        draft.edit.focused = focused;
        draft.replaced();
        draft.mode = Mode::Writing {
            reply: before.reply,
        };
        self.picker = None;
        if before.send_failed {
            draft.state = DraftState::Unsaved { due_s: 0.0 };
            self.failed = Some(key);
        }
        // A tray tile that finished copying during the edit waited for it
        // to end (an edit is never saved).
        if !draft.state.unsaved() {
            return Some(Vec::new());
        }
        Some(self.save(key).into_iter().collect())
    }

    /// Save on an edit: the edit is queued as `local` and the field as it
    /// was comes back. An edit emptied of text asks to delete the message
    /// instead, and stays open until that is answered.
    pub(super) fn save_edit(&mut self, local: Uuid) -> Sent {
        let Some((key, draft)) = self.shown() else {
            return Sent::Nothing;
        };
        let Some(id) = draft.mode.editing() else {
            return Sent::Nothing;
        };
        if draft.state == DraftState::Loading {
            return Sent::Nothing;
        }
        if draft.edit.text.trim().is_empty() {
            return Sent::DeleteInstead { key, id };
        }
        if draft.wire_chars > MAX_MESSAGE_CHARS {
            return Sent::TooLong;
        }
        let text = mentions::wire(&draft.edit);
        let save = self.cancel_edit().unwrap_or_default().pop();
        Sent::Edited {
            edit: Effect::Edit {
                key,
                id,
                text,
                local,
            },
            save,
        }
    }

    /// Saves `key`'s draft now: its reply changed.
    fn reply_changed(&mut self, key: TimelineKey) -> Vec<Effect> {
        if let Some(draft) = self.drafts.get_mut(&key) {
            draft.state = DraftState::Unsaved { due_s: 0.0 };
        }
        self.save(key).into_iter().collect()
    }
}

#[cfg(test)]
mod tests;
