//! Live-window local echoes from committed SDK operations; no native composer or send policy lives here.

use crate::{
    TimelineItem, timeline_item::ReplyOriginal, timeline_window::reply_originals::Originals,
};
use imbl::Vector;
use papo_sdk::{
    AccountId, InvalidMessageKey, MessageTarget, OperationKind, OperationState, OutgoingSnapshot,
    Uuid,
};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn append(
    items: &mut Vector<Arc<TimelineItem>>,
    outgoing: &OutgoingSnapshot,
    channel: Uuid,
    previous: &Vector<Arc<TimelineItem>>,
    history: &papo_sdk::HistorySnapshot,
    cache_scope: Option<(AccountId, Uuid)>,
) -> Result<(), InvalidMessageKey> {
    let mut originals = Originals::new(history, cache_scope);
    for item in items.iter() {
        if let TimelineItem::Message {
            id,
            message,
            content,
            ..
        } = item.as_ref()
        {
            originals.show(*id, message, content);
        }
    }
    let reusable: BTreeMap<_, _> = previous
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::LocalEcho { id, .. } => Some((*id, item)),
            _ => None,
        })
        .collect();
    for operation in &outgoing.operations {
        if operation.channel_id != channel
            || !matches!(operation.kind.as_ref(), OperationKind::Message(_))
        {
            continue;
        }
        if operation.state == OperationState::Sent && is_sent_echo_retired(operation, history) {
            continue;
        }
        let OperationKind::Message(message) = operation.kind.as_ref() else {
            continue;
        };
        let reply = match message.reply_to {
            Some(target) => Some(original(&originals, outgoing, target)?),
            None => None,
        };
        let value = TimelineItem::LocalEcho {
            id: operation.id,
            operation: Arc::clone(operation),
            reply,
        };
        let item = reusable
            .get(&operation.id)
            .filter(|old| old.as_ref() == &value)
            .map_or_else(|| Arc::new(value), |old| Arc::clone(old));
        items.push_back(item);
    }
    Ok(())
}

/// The original an echo replies to: a confirmed create by its backend ID, an unconfirmed one
/// by its local identity.
fn original(
    originals: &Originals<'_>,
    outgoing: &OutgoingSnapshot,
    target: MessageTarget,
) -> Result<ReplyOriginal, InvalidMessageKey> {
    match target {
        MessageTarget::Remote(id) => originals.of(id),
        MessageTarget::Local(local) => {
            let confirmed = outgoing
                .operations
                .iter()
                .find(|op| op.id == local)
                .and_then(|op| op.message_id);
            match confirmed {
                Some(id) => originals.of(id),
                None => Ok(ReplyOriginal::Unsent { local }),
            }
        }
    }
}

fn is_sent_echo_retired(
    operation: &papo_sdk::Operation,
    history: &papo_sdk::HistorySnapshot,
) -> bool {
    if operation.history_projected {
        return true;
    }
    let visible_in_history = operation
        .message
        .as_ref()
        .and_then(|message| papo_sdk::MessageKey::from_message(message).ok())
        .is_some_and(|point| {
            history
                .coverage()
                .and_then(|coverage| coverage.resolve(point))
                .is_ok_and(|resolved| resolved.index.is_some())
        });
    visible_in_history
        || operation
            .message_id
            .is_some_and(|id| history.tombstones.contains(&id))
}
