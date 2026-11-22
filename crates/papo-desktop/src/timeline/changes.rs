//! Your edits and deletions in flight, as their messages show them
//! (`docs/design.md` §11): the SDK's unsettled operation on each message
//! item (`pending`), sorted into what the row draws, with an edit's text
//! parsed once per operation image, never in a view.
//!
//! - Saving an edit: its text at once, quieter, the clock in the time slot.
//! - An edit that failed (or may not have arrived): its text, ⚠ in the time
//!   slot, whose menu is Retry and Discard edit.
//! - Deleting: the row at half opacity until the server confirms.
//! - A deletion that failed (or may not have arrived): the row as it was,
//!   with one danger line under it, Try again and Dismiss.
//!
//! A change the server confirmed draws as the message history now holds.

use std::collections::HashMap;
use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Operation, OperationKind, OperationState, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;

/// Where a change to a message stands, as its row shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChangeState {
    /// An edit on its way.
    Saving,
    /// An edit confirmed that history does not show yet: its text, as
    /// any message's.
    Saved,
    /// An edit refused, or cut off on its way.
    EditFailed,
    /// A deletion on its way, or confirmed and not yet gone.
    Deleting,
    /// A deletion refused, or cut off on its way.
    DeleteFailed,
}

/// One message's change.
pub(crate) struct Change {
    /// The SDK's image it was built from: a new one rebuilds it.
    operation: Arc<Operation>,
    /// The operation's local identity, which Retry and Discard name.
    pub(crate) local: Uuid,
    pub(crate) state: ChangeState,
    /// An edit's text, as written (wire form) and parsed.
    pub(crate) edit: Option<(String, Arc<MessageContent>)>,
}

/// The changes among a timeline's message items, by message ID.
#[derive(Default)]
pub(crate) struct Changes(HashMap<Uuid, Change>);

impl Changes {
    /// Message `id`'s change, if one is in flight.
    pub(crate) fn get(&self, id: Uuid) -> Option<&Change> {
        self.0.get(&id)
    }

    /// Follows `items`: keeps the changes whose operation image is the
    /// same, builds the new ones, forgets the settled.
    pub(crate) fn refresh(&mut self, items: &Vector<Arc<TimelineItem>>) {
        let mut next = HashMap::new();
        for item in items {
            let TimelineItem::Message {
                id,
                pending: Some(operation),
                ..
            } = item.as_ref()
            else {
                continue;
            };
            let kept = self
                .0
                .remove(id)
                .filter(|change| Arc::ptr_eq(&change.operation, operation));
            if let Some(change) = kept.or_else(|| build(operation)) {
                next.insert(*id, change);
            }
        }
        self.0 = next;
    }
}

fn build(operation: &Arc<Operation>) -> Option<Change> {
    let failed = matches!(
        operation.state,
        OperationState::Failed | OperationState::Unknown
    );
    let (state, edit) = match operation.kind.as_ref() {
        OperationKind::Edit { content, .. } => {
            let state = if failed {
                ChangeState::EditFailed
            } else if operation.state == OperationState::Sent {
                ChangeState::Saved
            } else {
                ChangeState::Saving
            };
            let parsed = Arc::new(MessageContent::parse(content));
            (state, Some((content.clone(), parsed)))
        }
        OperationKind::Delete { .. } if failed => (ChangeState::DeleteFailed, None),
        OperationKind::Delete { .. } => (ChangeState::Deleting, None),
        _ => return None,
    };
    Some(Change {
        operation: Arc::clone(operation),
        local: operation.id,
        state,
        edit,
    })
}
