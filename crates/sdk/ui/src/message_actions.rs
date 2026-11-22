//! What this account may do with one message (reply, edit, delete as author or moderator), and
//! which message "edit my newest message" opens. From authorship and the SDK's channel
//! affordances; never a substitute for the backend's own authorization. Menus, keys and words
//! are the client's.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, Operation, OperationKind, OperationState, Uuid, authored_by_other};

use crate::{ChannelCapabilities, TimelineItem};

/// The actions one message offers. An action the account may not take is absent, never
/// disabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MessageActions {
    /// Reply to it.
    pub reply: bool,
    /// Edit its text: only the author's own messages.
    pub edit: bool,
    /// Delete it, and in whose name.
    pub delete: Option<DeleteAs>,
}

/// Whose deletion a delete is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeleteAs {
    /// The author deletes their own message.
    Author,
    /// A moderator deletes someone else's; the backend records it in the audit log.
    Moderator,
}

/// A message "edit my newest message" opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTarget {
    /// Its backend ID.
    pub id: Uuid,
    /// The text to edit: an unsettled edit's, else the message's own (wire form, with
    /// canonical mention tokens).
    pub content: String,
}

/// The actions `message` offers to `account_user`. `pending` is the item's unsettled edit or
/// deletion; `capabilities` are the channel's, or `None` when they are unknown (permission
/// data not confirmed yet, or a DM read without the `CapabilityService`). Unknown fails
/// closed: it offers only deleting your own message, which the backend allows even after
/// access is lost; no reply, no edit and no moderator deletion.
#[must_use]
pub fn message_actions(
    message: &Message,
    pending: Option<&Operation>,
    account_user: Uuid,
    capabilities: Option<ChannelCapabilities>,
) -> MessageActions {
    if pending.is_some_and(is_deleting) {
        return MessageActions::default();
    }
    let own = is_own(message, account_user);
    let delete = if own {
        Some(DeleteAs::Author)
    } else if capabilities.is_some_and(|capabilities| capabilities.delete_others) {
        Some(DeleteAs::Moderator)
    } else {
        None
    };
    MessageActions {
        reply: capabilities.is_some_and(|capabilities| capabilities.send),
        edit: own && capabilities.is_some_and(|capabilities| capabilities.can_edit(true)),
        delete,
    }
}

/// The newest message in `items` that `account_user` may edit, with the text to edit. Unsent
/// messages are skipped (they are retried or discarded, not edited), and so are messages
/// being deleted. Unknown `capabilities` fail closed, as in [`message_actions`].
#[must_use]
pub fn newest_editable(
    items: &Vector<Arc<TimelineItem>>,
    account_user: Uuid,
    capabilities: Option<ChannelCapabilities>,
) -> Option<EditTarget> {
    if !capabilities.is_some_and(|capabilities| capabilities.can_edit(true)) {
        return None;
    }
    items.iter().rev().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id,
            message,
            pending,
            ..
        } if is_own(message, account_user) => {
            let pending = pending.as_deref();
            if pending.is_some_and(is_deleting) {
                return None;
            }
            let edited = pending.and_then(|operation| match operation.kind.as_ref() {
                OperationKind::Edit { content, .. } => Some(content.clone()),
                _ => None,
            });
            Some(EditTarget {
                id: *id,
                content: edited.unwrap_or_else(|| message.content.clone().unwrap_or_default()),
            })
        }
        TimelineItem::LocalEcho { operation, .. } if operation.state == OperationState::Sent => {
            let OperationKind::Message(outgoing) = operation.kind.as_ref() else {
                return None;
            };
            let content = operation
                .message
                .as_ref()
                .and_then(|receipt| receipt.content.clone())
                .unwrap_or_else(|| outgoing.content.clone());
            operation.message_id.map(|id| EditTarget { id, content })
        }
        _ => None,
    })
}

/// Written by this account itself: not someone else's, and not a webhook or bridged author's
/// that happens to carry the account's ID. The one rule for "your message" in this crate.
pub(crate) fn is_own(message: &Message, account_user: Uuid) -> bool {
    !authored_by_other(message, account_user)
        && message.webhook_id.is_none()
        && message.external_author.is_none()
}

/// A deletion that has not failed: the message is going away.
fn is_deleting(operation: &Operation) -> bool {
    matches!(operation.kind.as_ref(), OperationKind::Delete { .. })
        && operation.state != OperationState::Failed
}

#[cfg(test)]
mod tests;
