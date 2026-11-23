//! Which actions a message offers and which message ↑ edits, from synthetic items.

use super::*;
use crate::message_content::MessageContent;
use papo_sdk::{MessageTarget, OutgoingMessage};

const ME: u128 = 1;
const OTHER: u128 = 2;

fn message(id: u128, author: u128, content: &str) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        author_id: Some(Uuid::from_u128(author).to_string()),
        content: Some(content.to_owned()),
        ..Message::default()
    })
}

fn operation(id: u128, kind: OperationKind, state: OperationState) -> Arc<Operation> {
    Arc::new(Operation {
        id: Uuid::from_u128(id),
        channel_id: Uuid::from_u128(100),
        kind: Arc::new(kind),
        state,
        attempts: 0,
        depends_on: None,
        failure: None,
        message_id: None,
        history_projected: false,
        message: None,
        created_at_us: 0,
    })
}

fn item(message: Arc<Message>, pending: Option<Arc<Operation>>) -> Arc<TimelineItem> {
    let id = message.id.as_deref().unwrap().parse().unwrap();
    Arc::new(TimelineItem::Message {
        id,
        content: Arc::new(MessageContent::parse("")),
        message,
        reply: None,
        pending,
    })
}

fn moderator() -> ChannelCapabilities {
    ChannelCapabilities {
        send: true,
        delete_others: true,
        ..ChannelCapabilities::default()
    }
}

/// A member who may write in the channel.
fn writer() -> ChannelCapabilities {
    ChannelCapabilities {
        send: true,
        ..ChannelCapabilities::default()
    }
}

fn me() -> Uuid {
    Uuid::from_u128(ME)
}

#[test]
fn authors_edit_and_delete_their_own_and_moderators_delete_others() {
    let mine = message(10, ME, "mine");
    let theirs = message(11, OTHER, "theirs");
    let reader = ChannelCapabilities {
        send: true,
        ..ChannelCapabilities::default()
    };
    assert_eq!(
        message_actions(&mine, None, None, me(), Some(reader)),
        MessageActions {
            reply: true,
            edit: true,
            delete: Some(DeleteAs::Author),
            ..MessageActions::default()
        }
    );
    assert_eq!(
        message_actions(&theirs, None, None, me(), Some(reader)),
        MessageActions {
            reply: true,
            ..MessageActions::default()
        }
    );
    assert_eq!(
        message_actions(&theirs, None, None, me(), Some(moderator())).delete,
        Some(DeleteAs::Moderator)
    );
    let muted = ChannelCapabilities::default();
    assert!(!message_actions(&theirs, None, None, me(), Some(muted)).reply);
}

#[test]
fn unknown_permissions_fail_closed_but_for_deleting_your_own() {
    let theirs = message(11, OTHER, "theirs");
    assert_eq!(
        message_actions(&theirs, None, None, me(), None),
        MessageActions::default()
    );
    let mine = message(10, ME, "mine");
    assert_eq!(
        message_actions(&mine, None, None, me(), None),
        MessageActions {
            delete: Some(DeleteAs::Author),
            ..MessageActions::default()
        }
    );
    let items: Vector<_> = [item(mine, None)].into_iter().collect();
    assert_eq!(newest_editable(&items, me(), None), None);
}

#[test]
fn a_webhook_message_naming_you_is_not_yours() {
    let mut hooked = Message::clone(&message(10, ME, "relayed"));
    hooked.webhook_id = Some(Uuid::from_u128(9).to_string());
    let actions = message_actions(&hooked, None, None, me(), Some(moderator()));
    assert!(!actions.edit);
    assert_eq!(actions.delete, Some(DeleteAs::Moderator));
}

#[test]
fn a_message_being_deleted_offers_nothing_until_the_deletion_fails() {
    let mine = message(10, ME, "mine");
    let target = MessageTarget::Remote(Uuid::from_u128(10));
    let deleting = operation(
        20,
        OperationKind::Delete { target },
        OperationState::Sending,
    );
    assert_eq!(
        message_actions(&mine, Some(&deleting), None, me(), None),
        MessageActions::default()
    );
    let failed = operation(20, OperationKind::Delete { target }, OperationState::Failed);
    assert!(message_actions(&mine, Some(&failed), None, me(), Some(writer())).edit);
}

#[test]
fn pin_and_unpin_need_known_pins_and_their_permission() {
    let theirs = message(11, OTHER, "theirs");
    let pinner = ChannelCapabilities {
        read: true,
        pin: true,
        unpin: true,
        ..ChannelCapabilities::default()
    };
    let actions = |pinned, capabilities| message_actions(&theirs, None, pinned, me(), capabilities);
    assert_eq!(actions(Some(false), Some(pinner)).pin, Some(PinChange::Pin));
    assert_eq!(
        actions(Some(true), Some(pinner)).pin,
        Some(PinChange::Unpin)
    );
    assert_eq!(actions(None, Some(pinner)).pin, None);
    assert_eq!(actions(Some(false), Some(writer())).pin, None);
    assert_eq!(actions(Some(true), None).pin, None);
    let pins: Vector<_> = [Arc::clone(&theirs)].into_iter().collect();
    assert_eq!(pinned(Some(&pins), Uuid::from_u128(11)), Some(true));
    assert_eq!(pinned(Some(&pins), Uuid::from_u128(10)), Some(false));
    assert_eq!(pinned(None, Uuid::from_u128(11)), None);
}

#[test]
fn reacting_needs_the_add_permission_and_seeing_who_reacted_needs_reactions_and_read() {
    let mut reacted = Message::clone(&message(11, OTHER, "theirs"));
    reacted.reactions = Some(vec![
        papo_sdk::operations::interactions::MessageReactionSummary {
            emoji_id: None,
            unicode: Some("👍".into()),
            count: Some(1),
        },
    ]);
    let member = ChannelCapabilities {
        read: true,
        add_reaction: true,
        ..ChannelCapabilities::default()
    };
    let actions = message_actions(&reacted, None, None, me(), Some(member));
    assert!(actions.react && actions.view_reactions);
    let silent = message_actions(
        &message(11, OTHER, "theirs"),
        None,
        None,
        me(),
        Some(member),
    );
    assert!(silent.react && !silent.view_reactions);
    assert!(!message_actions(&reacted, None, None, me(), None).view_reactions);
    let blind = ChannelCapabilities {
        read: false,
        ..member
    };
    assert!(!message_actions(&reacted, None, None, me(), Some(blind)).view_reactions);
    let muted = ChannelCapabilities {
        add_reaction: false,
        ..member
    };
    assert!(!message_actions(&reacted, None, None, me(), Some(muted)).react);
}

#[test]
fn up_edits_the_newest_own_sent_message_with_its_unsettled_text() {
    let edit = operation(
        20,
        OperationKind::Edit {
            target: MessageTarget::Remote(Uuid::from_u128(12)),
            content: "retyped".into(),
            embeds: None,
        },
        OperationState::Failed,
    );
    let unsent = operation(
        21,
        OperationKind::Message(OutgoingMessage {
            content: "queued".into(),
            ..OutgoingMessage::default()
        }),
        OperationState::Queued,
    );
    let items: Vector<_> = [
        item(message(10, ME, "older"), None),
        item(message(12, ME, "original"), Some(edit)),
        item(message(13, OTHER, "theirs"), None),
        Arc::new(TimelineItem::LocalEcho {
            id: Uuid::from_u128(21),
            operation: unsent,
            reply: None,
        }),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        newest_editable(&items, me(), Some(writer())),
        Some(EditTarget {
            id: Uuid::from_u128(12),
            content: "retyped".into(),
        })
    );
}

#[test]
fn up_takes_a_sent_echo_and_skips_a_message_being_deleted() {
    let mut sent = Operation::clone(&operation(
        21,
        OperationKind::Message(OutgoingMessage {
            content: "just sent".into(),
            ..OutgoingMessage::default()
        }),
        OperationState::Sent,
    ));
    sent.message_id = Some(Uuid::from_u128(14));
    let deleting = operation(
        22,
        OperationKind::Delete {
            target: MessageTarget::Remote(Uuid::from_u128(12)),
        },
        OperationState::Queued,
    );
    let items: Vector<_> = [
        item(message(10, ME, "older"), None),
        item(message(12, ME, "going"), Some(deleting)),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        newest_editable(&items, me(), Some(writer())).map(|target| target.id),
        Some(Uuid::from_u128(10))
    );
    let mut with_echo = items;
    with_echo.push_back(Arc::new(TimelineItem::LocalEcho {
        id: Uuid::from_u128(21),
        operation: Arc::new(sent),
        reply: None,
    }));
    assert_eq!(
        newest_editable(&with_echo, me(), Some(writer())),
        Some(EditTarget {
            id: Uuid::from_u128(14),
            content: "just sent".into(),
        })
    );
    let read_only = ChannelCapabilities {
        direct: true,
        ..ChannelCapabilities::default()
    };
    assert_eq!(newest_editable(&with_echo, me(), Some(read_only)), None);
}
