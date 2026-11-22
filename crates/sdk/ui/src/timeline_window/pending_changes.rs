//! This account's edits and deletions that history has not settled yet, by the message they
//! change. Reads the committed outgoing image only; dispatch and settlement are `papo-sdk`'s.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{MessageTarget, Operation, OperationKind, OperationState, OutgoingSnapshot, Uuid};

/// The newest unsettled edit or deletion of each message in `channel`, by backend message ID.
/// An operation is settled once it is Sent and history has taken it in; until then (Queued,
/// Sending, Failed, Unknown, or Sent but not yet settled) the message's item carries it. An
/// operation on a create that has no backend ID yet changes no message item.
pub(crate) fn by_message(
    outgoing: &OutgoingSnapshot,
    channel: Uuid,
) -> HashMap<Uuid, Arc<Operation>> {
    let created: HashMap<Uuid, Uuid> = outgoing
        .operations
        .iter()
        .filter(|op| matches!(op.kind.as_ref(), OperationKind::Message(_)))
        .filter_map(|op| op.message_id.map(|message| (op.id, message)))
        .collect();
    let mut pending = HashMap::new();
    for operation in &outgoing.operations {
        if operation.channel_id != channel
            || (operation.state == OperationState::Sent && operation.history_projected)
        {
            continue;
        }
        let target = match operation.kind.as_ref() {
            OperationKind::Edit { target, .. } | OperationKind::Delete { target } => target,
            _ => continue,
        };
        let message = match target {
            MessageTarget::Remote(id) => Some(*id),
            MessageTarget::Local(local) => operation.message_id.or(created.get(local).copied()),
        };
        if let Some(message) = message {
            // Insertion order: a later operation on the same message replaces an earlier one.
            pending.insert(message, Arc::clone(operation));
        }
    }
    pending
}

#[cfg(test)]
mod tests {
    //! Which operation each message item carries, from synthetic committed images.

    use super::*;
    use papo_sdk::OutgoingMessage;

    fn channel() -> Uuid {
        Uuid::from_u128(100)
    }

    fn operation(
        id: u128,
        kind: OperationKind,
        state: OperationState,
        projected: bool,
    ) -> Arc<Operation> {
        Arc::new(Operation {
            id: Uuid::from_u128(id),
            channel_id: channel(),
            kind: Arc::new(kind),
            state,
            attempts: 0,
            depends_on: None,
            failure: None,
            message_id: None,
            history_projected: projected,
            message: None,
            created_at_us: 0,
        })
    }

    fn edit(target: MessageTarget) -> OperationKind {
        OperationKind::Edit {
            target,
            content: "synthetic".into(),
            embeds: None,
        }
    }

    fn image(operations: Vec<Arc<Operation>>) -> OutgoingSnapshot {
        OutgoingSnapshot {
            account: None,
            version: 1,
            operations,
            upload_progress: None,
            fault: None,
        }
    }

    fn remote(id: u128) -> MessageTarget {
        MessageTarget::Remote(Uuid::from_u128(id))
    }

    #[test]
    fn an_edit_stays_until_sent_and_settled() {
        for (state, projected, carried) in [
            (OperationState::Queued, false, true),
            (OperationState::Sending, false, true),
            (OperationState::Failed, false, true),
            (OperationState::Unknown, false, true),
            (OperationState::Sent, false, true),
            (OperationState::Sent, true, false),
        ] {
            let pending = by_message(
                &image(vec![operation(1, edit(remote(7)), state, projected)]),
                channel(),
            );
            assert_eq!(
                pending.contains_key(&Uuid::from_u128(7)),
                carried,
                "{state:?}, projected {projected}"
            );
        }
    }

    #[test]
    fn the_newest_change_wins_and_other_channels_and_kinds_do_not_count() {
        let mut elsewhere = (*operation(3, edit(remote(7)), OperationState::Queued, false)).clone();
        elsewhere.channel_id = Uuid::from_u128(101);
        let pending = by_message(
            &image(vec![
                operation(1, edit(remote(7)), OperationState::Failed, false),
                operation(
                    2,
                    OperationKind::Delete { target: remote(7) },
                    OperationState::Queued,
                    false,
                ),
                Arc::new(elsewhere),
                operation(
                    4,
                    OperationKind::Message(OutgoingMessage::default()),
                    OperationState::Queued,
                    false,
                ),
            ]),
            channel(),
        );
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[&Uuid::from_u128(7)].id, Uuid::from_u128(2));
    }

    #[test]
    fn a_change_to_a_local_create_follows_its_confirmed_id() {
        let local = MessageTarget::Local(Uuid::from_u128(1));
        let mut create = (*operation(
            1,
            OperationKind::Message(OutgoingMessage::default()),
            OperationState::Queued,
            false,
        ))
        .clone();
        let unconfirmed = by_message(
            &image(vec![
                Arc::new(create.clone()),
                operation(2, edit(local), OperationState::Queued, false),
            ]),
            channel(),
        );
        assert!(unconfirmed.is_empty(), "no message item to change yet");
        create.state = OperationState::Sent;
        create.message_id = Some(Uuid::from_u128(7));
        let confirmed = by_message(
            &image(vec![
                Arc::new(create),
                operation(2, edit(local), OperationState::Queued, false),
            ]),
            channel(),
        );
        assert_eq!(confirmed[&Uuid::from_u128(7)].id, Uuid::from_u128(2));
    }
}
