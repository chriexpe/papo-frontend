//! A message's hover bar from the shell's side (`docs/design.md` §10): its
//! presses, and its quick-reaction strip, which Add reaction (or + on a
//! focused message) stretches the bar into and which a reaction, Escape or
//! Add reaction again folds back. While it is stretched, More emoji leads
//! it (§16.2), ← and → on the message choose a spot, and Enter or Space
//! reacts with the emoji or opens the picker.

use papo_sdk::Uuid;
use rune::{Key, KeyEvent, Tx};

use super::Shell;
use super::activity::Stretched;
use crate::menu::Placement;
use crate::quick_strip::StripSpot;
use crate::timeline::TimelineKey;
use crate::timeline::hover_bar::{BarPress, more_key};

impl Shell {
    /// A press on message `id`'s bar.
    pub(super) fn bar_press(&mut self, key: TimelineKey, id: Uuid, press: BarPress, tx: &mut Tx) {
        match press {
            BarPress::React { emoji } => {
                self.stretched = None;
                self.toggle_reaction(key, id, &emoji);
            }
            BarPress::AddReaction => self.toggle_strip(key, id),
            BarPress::MoreEmoji => {
                self.stretched = None;
                self.more_emoji_pressed(key, id, tx);
            }
            BarPress::Reply => {
                self.reply_to(key, id, tx);
            }
            BarPress::Edit => {
                self.edit(key, id, tx);
            }
            BarPress::More => {
                self.stretched = None;
                let trigger = pointer_trigger(tx);
                self.open_message_menu(key, id, Placement::BelowEnd(more_key(id)), trigger);
            }
        }
    }

    /// Add reaction, or + on the focused message: the bar stretches into
    /// the strip, or folds back when it already had.
    pub(super) fn toggle_strip(&mut self, key: TimelineKey, id: Uuid) {
        let open = self
            .stretched
            .is_some_and(|stretched| stretched.key == key && stretched.id == id);
        let strip = !self.quick_reactions(key.account).is_empty() && self.may_react(key, id);
        self.stretched = (!open && strip).then_some(Stretched {
            key,
            id,
            chosen: StripSpot::FIRST,
        });
    }

    /// The pointer came onto message `message`: a bar stretched on another
    /// message folds, as the pointer has left it. Returns whether one did.
    pub(super) fn fold_strip_for(&mut self, key: TimelineKey, message: Uuid) -> bool {
        let elsewhere = self
            .stretched
            .is_some_and(|stretched| stretched.key != key || stretched.id != message);
        if elsewhere {
            self.stretched = None;
        }
        elsewhere
    }

    /// The bar's top three, 1 2 3 on the focused message: reacts with the
    /// `index`th quick reaction, or takes yours back.
    pub(super) fn quick_react(&mut self, key: TimelineKey, id: Uuid, index: usize) -> bool {
        let Some(emoji) = self.quick_reactions(key.account).get(index).cloned() else {
            return false;
        };
        self.toggle_reaction(key, id, &emoji)
    }

    /// A key on message `id` while its bar is stretched: ← → choose, Enter
    /// or Space reacts, or opens the picker on More emoji (Escape folds it,
    /// in its turn: `OnTop`). Returns whether it was taken.
    pub(super) fn strip_key(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        event: &KeyEvent,
        tx: &mut Tx,
    ) -> bool {
        let Some(mut stretched) = self
            .stretched
            .filter(|stretched| stretched.key == key && stretched.id == id)
        else {
            return false;
        };
        if event.mods.shift {
            return false;
        }
        let count = self.quick_reactions(key.account).len();
        let pressed = match event.key {
            Key::Right => {
                stretched.chosen = stretched.chosen.step(count, true);
                false
            }
            Key::Left => {
                stretched.chosen = stretched.chosen.step(count, false);
                false
            }
            // Taken either way: Enter here never opens the menu.
            Key::Enter => true,
            Key::Text(ref space) if space == " " => true,
            _ => return false,
        };
        if pressed {
            self.stretched = None;
            match stretched.chosen {
                StripSpot::More => self.more_emoji_pressed(key, id, tx),
                StripSpot::Emoji(index) => {
                    self.quick_react(key, id, index);
                }
            }
            return true;
        }
        self.stretched = Some(stretched);
        true
    }
}

/// What gets the keyboard back from a menu the pointer opened: what had
/// it, else the composer.
pub(super) fn pointer_trigger(tx: &Tx) -> String {
    tx.focused()
        .map_or_else(|| crate::composer::COMPOSER_KEY.to_owned(), str::to_owned)
}
