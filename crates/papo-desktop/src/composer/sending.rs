//! Send and what follows when the SDK refuses it: the shown field's text
//! and tray leave as one message, and come back when it is not queued.

use papo_sdk::Uuid;
use papo_sdk_ui::message_content::DecodedComposerMentions;

use super::mode::{Mode, Reply};
use super::tray::Tile;
use super::{Composer, DraftState, Effect, Sent, TimelineKey, mentions};

impl Composer {
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
        let tiles = draft.tray.take();
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
            tiles,
            local,
            seq,
        })
    }

    /// The SDK did not queue `decoded` and `tiles` for `key`: they go back
    /// in the field and the tray, before whatever was added since, and are
    /// saved again.
    pub(crate) fn send_failed(
        &mut self,
        key: TimelineKey,
        decoded: DecodedComposerMentions,
        reply_to: Option<Uuid>,
        mention: bool,
        tiles: Vec<Tile>,
    ) -> Vec<Effect> {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Vec::new();
        };
        // The tray is the draft's, edit or not (§11: only the text is edited).
        draft.tray.restore(tiles);
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
}
