//! Live-window local echoes from committed SDK operations; no native composer or send policy lives here.

use crate::TimelineItem;
use imbl::Vector;
use papo_sdk::{OperationKind, OperationState, OutgoingSnapshot, Uuid};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn append(
    items: &mut Vector<Arc<TimelineItem>>,
    outgoing: &OutgoingSnapshot,
    channel: Uuid,
    previous: &Vector<Arc<TimelineItem>>,
) {
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
        if operation.state == OperationState::Sent {
            continue;
        }
        let value = TimelineItem::LocalEcho {
            id: operation.id,
            operation: Arc::clone(operation),
        };
        let item = reusable
            .get(&operation.id)
            .filter(|old| old.as_ref() == &value)
            .map_or_else(|| Arc::new(value), |old| Arc::clone(old));
        items.push_back(item);
    }
}
