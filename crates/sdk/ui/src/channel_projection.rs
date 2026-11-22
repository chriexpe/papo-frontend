//! Ordered channel metadata derived from admitted SDK records. Reading and badges are separate overlays.

use std::sync::Arc;

use futures_util::StreamExt;
use papo_sdk::{ResourceSnapshot, Uuid};
use tokio::sync::watch;

use crate::{
    channel_list::{ChannelListItem, ChannelListSubscription, ChannelListUpdate},
    resource_list::{self, List},
};

pub(crate) fn subscribe(source: watch::Receiver<Arc<ResourceSnapshot>>) -> ChannelListSubscription {
    let subscription = resource_list::subscribe(source, List::Channels, project);
    ChannelListSubscription {
        version: subscription.version,
        generation: subscription.generation,
        snapshot: subscription.snapshot,
        stream: Box::pin(subscription.stream.map(|update| ChannelListUpdate {
            version: update.version,
            generation: update.generation,
            diffs: update.diffs,
        })),
    }
}

fn project(snapshot: &ResourceSnapshot, id: Uuid) -> ChannelListItem {
    let channel = snapshot
        .channels
        .get(id)
        .expect("channel ID belongs to this immutable table");
    ChannelListItem {
        id,
        name: channel.name.clone(),
        kind: channel.kind,
        position: channel.position,
        topic: channel.topic.clone(),
        parent_id: channel.parent_id,
        notification_settings: channel.notification_settings.clone(),
        created_at: channel.created_at.clone(),
        unread: None,
        badges: None,
    }
}

#[cfg(test)]
mod tests;
