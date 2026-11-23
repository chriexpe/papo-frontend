//! Replying and editing from the shell's side (`docs/design.md` §11): what
//! starts them (a focused message's R and E, ↑ in an empty field), the
//! bar's presses, looking up a restored reply's original, and queueing an
//! edit. The composer keeps the mode; the SDK decides what may be done
//! (`message_actions`, `newest_editable`).

use papo_sdk::permissions::PermissionDataError;
use papo_sdk::{OperationKind, ResourceSnapshot, Uuid};
use papo_sdk_ui::message_content::{DecodedComposerMentions, decode_composer_mentions};
use papo_sdk_ui::{
    ChannelCapabilities, MessageActions, ReplyOriginal, TimelineItem, capabilities, display_name,
    message_actions, newest_editable,
};
use rune::Tx;

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::sdk_bridge::Sdk;
use crate::timeline::rows::anchor_key;
use crate::timeline::{OpenTimeline, TimelineKey};

impl Shell {
    /// R on message `id` of timeline `key`: a reply to it opens in the
    /// composer, which takes the keyboard.
    pub(crate) fn reply_to(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) -> bool {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        let Some((actions, original)) = item_actions(open, id) else {
            return false;
        };
        if !actions.reply {
            return false;
        }
        let mention = self.writing.mention_when_replying;
        let effects = self.composer.start_reply(key, id, Some(original), mention);
        self.run_composer(effects, Some(tx));
        tx.scroll_into_view(anchor_key(id));
        tx.focus(COMPOSER_KEY, false);
        true
    }

    /// E on your message `id` of timeline `key`: its text comes into the
    /// composer to be edited.
    pub(crate) fn edit(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) -> bool {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        let Some((actions, _)) = item_actions(open, id) else {
            return false;
        };
        if !actions.edit {
            return false;
        }
        let Some(text) = editable_text(open, id) else {
            return false;
        };
        self.start_edit(key, id, &text, tx)
    }

    /// ↑ in an empty field: your newest message in the conversation is
    /// edited. Only where the conversation shows its newest messages: an
    /// older stretch of it does not know which is newest.
    pub(crate) fn edit_newest(&mut self, tx: &mut Tx) -> bool {
        let Some((key, _)) = self.composer.shown() else {
            return false;
        };
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        if !open.follows_newest() {
            return false;
        }
        let user = key.account.user();
        let Some(target) = newest_editable(&open.items, user, channel_capabilities(open)) else {
            return false;
        };
        self.start_edit(key, target.id, &target.content, tx)
    }

    /// ✕ on the reply bar.
    pub(crate) fn reply_closed(&mut self) {
        let effects = self.composer.cancel_reply().unwrap_or_default();
        self.run_composer(effects, None);
    }

    /// ✕ on the edit bar.
    pub(crate) fn edit_closed(&mut self) {
        let effects = self.composer.cancel_edit().unwrap_or_default();
        self.run_composer(effects, None);
    }

    /// The @ switch on the reply bar.
    pub(crate) fn reply_mention_toggled(&mut self) {
        self.composer.toggle_reply_mention();
    }

    /// Looks up the originals of restored replies whose conversation is
    /// open: the bar names them once the SDK answers.
    pub(super) fn resolve_replies(&self) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        for (key, id) in self.composer.unresolved_replies() {
            let Some(handle) = self
                .tabs
                .shown()
                .filter(|open| open.key == key)
                .and_then(OpenTimeline::handle)
                .cloned()
            else {
                continue;
            };
            let updates = sdk.updates();
            // Off the window's thread: the lookup may read the whole cache.
            sdk.run(move |_client| async move {
                match handle.original(id) {
                    Ok(original) => {
                        updates.send(move |shell| shell.composer.reply_found(key, id, original));
                    }
                    Err(error) => log::debug!("a reply's original was not looked up: {error}"),
                }
            });
        }
    }

    /// Opens the edit of message `id` with its text `wire` (canonical
    /// mention tokens), named from the conversation's people.
    fn start_edit(&mut self, key: TimelineKey, id: Uuid, wire: &str, tx: &mut Tx) -> bool {
        let resources = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| open.resources.clone());
        let decoded = decode(resources.as_deref(), wire);
        let effects = self.composer.start_edit(key, id, decoded);
        self.run_composer(effects, Some(tx));
        tx.scroll_into_view(anchor_key(id));
        tx.focus(COMPOSER_KEY, false);
        true
    }
}

/// Queues the edit of message `id` in `key` to `text` (wire form) as
/// `local`. One the SDK refuses comes back into the composer as an edit.
pub(super) fn queue_edit(sdk: &Sdk<Shell>, key: TimelineKey, id: Uuid, text: String, local: Uuid) {
    let updates = sdk.updates();
    sdk.run(move |client| async move {
        let session = client.session(key.account);
        let resources = session
            .as_ref()
            .ok()
            .map(|session| session.subscribe_resources().borrow().clone());
        let queued = match session {
            Ok(session) => {
                let kind = OperationKind::Edit {
                    target: papo_sdk::MessageTarget::Remote(id),
                    content: text.clone(),
                    embeds: None,
                };
                session
                    .outgoing()
                    .enqueue(key.channel, local, kind, super::composer::now())
                    .await
            }
            Err(error) => Err(error.into()),
        };
        if let Err(error) = queued {
            log::warn!("an edit was not queued: {error}");
            let decoded = decode(resources.as_deref(), &text);
            updates.send(move |shell| {
                let effects = shell.composer.edit_came_back(key, id, decoded);
                shell.run_composer(effects, None);
                true
            });
        }
    });
}

/// The actions message `id` of `open` offers, and its original as a reply
/// would hold it; none when the conversation does not show it.
pub(super) fn item_actions(
    open: &OpenTimeline,
    id: Uuid,
) -> Option<(MessageActions, ReplyOriginal)> {
    let user = open.key.account.user();
    open.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current,
            message,
            content,
            pending,
            ..
        } if *current == id => {
            let actions = message_actions(
                message,
                pending.as_deref(),
                open.pins.pinned(id),
                user,
                channel_capabilities(open),
            );
            let original = ReplyOriginal::Held {
                id,
                message: message.clone(),
                content: content.clone(),
            };
            Some((actions, original))
        }
        _ => None,
    })
}

/// The text an edit of message `id` starts from: an unsettled edit's,
/// else the message's.
fn editable_text(open: &OpenTimeline, id: Uuid) -> Option<String> {
    open.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current,
            message,
            pending,
            ..
        } if *current == id => {
            let edited = pending.as_ref().and_then(|op| match op.kind.as_ref() {
                OperationKind::Edit { content, .. } => Some(content.clone()),
                _ => None,
            });
            Some(edited.unwrap_or_else(|| message.content.clone().unwrap_or_default()))
        }
        _ => None,
    })
}

/// The channel's affordances for the account, none while unknown.
pub(super) fn channel_capabilities(open: &OpenTimeline) -> Option<ChannelCapabilities> {
    let resources = open.resources.as_deref()?;
    let found: Result<ChannelCapabilities, PermissionDataError> =
        capabilities(resources, open.key.account.user(), open.key.channel);
    found.ok()
}

/// `wire` as the field shows it: each mention token the person's name.
fn decode(resources: Option<&ResourceSnapshot>, wire: &str) -> DecodedComposerMentions {
    decode_composer_mentions(wire, |id| {
        let user = resources?.users.get(id)?;
        display_name(user.nickname.as_deref(), user.username.as_deref())
    })
}
