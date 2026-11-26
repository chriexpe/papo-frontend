//! Reacting and pinning from the shell's side: one gate for every
//! reaction press (a chip, the bar's or the strip's emoji, 1 2 3) that
//! waits while a change of it is on its way and checks the permission,
//! and pinning and unpinning through the SDK, one change of a message at
//! a time. A refused one is said in the notice pill.

use std::collections::HashSet;

use papo_sdk::operations::reactions::{ReactionEmoji, ReactionIntent};
use papo_sdk::{MessageTarget, Uuid};
use papo_sdk_ui::{PinChange, reaction_chips};

use super::Shell;
use super::replies::item_actions;
use crate::notice::Refusal;
use crate::timeline::TimelineKey;

/// Pins and unpins on their way, by message: a second press waits for the
/// first (a control whose call is in flight waits).
#[derive(Default)]
pub(crate) struct Pinning(HashSet<(TimelineKey, Uuid)>);

impl Pinning {
    /// Whether a pin or unpin of message `id` is on its way.
    pub(crate) fn waits(&self, key: TimelineKey, id: Uuid) -> bool {
        self.0.contains(&(key, id))
    }
}

impl Shell {
    /// P on the focused message: pins it, or unpins it when pinned.
    /// Returns whether the key was taken.
    pub(super) fn toggle_pin(&mut self, key: TimelineKey, id: Uuid) -> bool {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        match item_actions(open, id).and_then(|(actions, _)| actions.pin) {
            Some(PinChange::Pin) => self.set_pin(key, id, true),
            Some(PinChange::Unpin) => self.set_pin(key, id, false),
            _ => return false,
        }
        true
    }

    /// P, or Pin or Unpin in the menu: pins message `id` (`pin`) or
    /// unpins it, if the account may and no pin of it is on its way. The
    /// pins follow from the SDK's pin events; a refusal is said.
    pub(super) fn set_pin(&mut self, key: TimelineKey, id: Uuid, pin: bool) {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return;
        };
        let wanted = if pin {
            PinChange::Pin
        } else {
            PinChange::Unpin
        };
        let allowed = item_actions(open, id).and_then(|(actions, _)| actions.pin);
        if allowed == Some(wanted) {
            self.send_pin(key, id, pin);
        }
    }

    /// Pins message `id` (`pin`) or unpins it through the SDK, unless a
    /// change of it is already on its way; a refusal is said.
    pub(super) fn send_pin(&mut self, key: TimelineKey, id: Uuid, pin: bool) {
        if !self.pinning.0.insert((key, id)) {
            return;
        }
        let Some(sdk) = &self.sdk else {
            self.pinning.0.remove(&(key, id));
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => {
                    let operations = session.operations();
                    if pin {
                        operations.pin_message(key.channel, id).await.map(|_| ())
                    } else {
                        operations.unpin_message(key.channel, id).await.map(|_| ())
                    }
                }
                Err(error) => Err(error.into()),
            };
            let refused = done.err().map(|error| {
                log::warn!("a pin change did not happen: {error}");
                if pin { Refusal::Pin } else { Refusal::Unpin }
            });
            updates.send_tx(move |shell, tx| {
                shell.pinning.0.remove(&(key, id));
                if let Some(refusal) = refused {
                    shell.refuse(refusal, tx);
                }
                true
            });
        });
    }

    /// Whether your reaction `emoji` to message `id` is there, when you may
    /// change it now: none without the permission, or while a change of it
    /// is on its way (the chip waits).
    pub(super) fn reacts_with(
        &self,
        key: TimelineKey,
        id: Uuid,
        emoji: &ReactionEmoji,
    ) -> Option<bool> {
        let open = self.tabs.shown().filter(|open| open.key == key)?;
        let message = open.message(id).map(|(message, _)| message.as_ref())?;
        let outgoing = self.outgoing(key.account);
        let chip = reaction_chips(message, &outgoing)
            .into_iter()
            .find(|chip| chip.emoji == *emoji);
        match chip {
            Some(chip) => chip
                .can_toggle(super::replies::channel_capabilities(open))
                .then_some(chip.mine),
            None => item_actions(open, id)
                .is_some_and(|(actions, _)| actions.react)
                .then_some(false),
        }
    }

    /// Whether you may react to message `id` of conversation `key`, shown now.
    pub(super) fn may_react(&self, key: TimelineKey, id: Uuid) -> bool {
        self.tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| item_actions(open, id))
            .is_some_and(|(actions, _)| actions.react)
    }

    /// A chip, the bar's or the strip's emoji, 1 2 3: your reaction `emoji`
    /// to message `id` is taken back when yours, else added; nothing while
    /// a change of it waits or the account may not. Returns whether it went.
    pub(crate) fn toggle_reaction(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        emoji: &ReactionEmoji,
    ) -> bool {
        let Some(mine) = self.reacts_with(key, id, emoji) else {
            return false;
        };
        self.react(key, id, emoji.clone(), !mine);
        true
    }

    /// A quick reaction or a chip: your reaction `emoji` to message `id`
    /// is added (`add`) or taken back. The chip waits for the SDK's answer;
    /// a refused one is said.
    pub(crate) fn react(&mut self, key: TimelineKey, id: Uuid, emoji: ReactionEmoji, add: bool) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let local = (self.local_ids)();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let admitted = match client.session(key.account) {
                Ok(session) => session
                    .reactions()
                    .set(ReactionIntent {
                        channel: key.channel,
                        message: MessageTarget::Remote(id),
                        emoji,
                        add,
                        local,
                        at: super::draft_writes::now(),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = admitted {
                log::warn!("a reaction was not queued: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(Refusal::Reaction, tx);
                    true
                });
            }
        });
    }
}
