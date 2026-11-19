//! Read and badge overlays for channel metadata; canonical evidence and policy stay in the SDK.

use futures_util::{StreamExt, stream};
use imbl::Vector;
use papo_sdk::{BadgesSnapshot, ReadingSnapshot};
use std::sync::Arc;
use tokio::sync::watch;

use crate::{
    channel_list::{ChannelListItem, ChannelListSubscription, ChannelListUpdate},
    list_subscription::{Event, Publisher},
};

fn overlay(
    metadata: &Vector<Arc<ChannelListItem>>,
    reading: &ReadingSnapshot,
    badges: &BadgesSnapshot,
    generation: papo_sdk::Generation,
    previous: &Vector<Arc<ChannelListItem>>,
) -> Vector<Arc<ChannelListItem>> {
    let reusable =
        crate::channel_projection::unique_ids(metadata.iter().map(|item| item.id.as_deref()))
            && crate::channel_projection::unique_ids(
                previous.iter().map(|item| item.id.as_deref()),
            );
    let by_id: std::collections::BTreeMap<_, _> = previous
        .iter()
        .filter_map(|item| item.id.as_deref().map(|id| (id, item)))
        .collect();
    metadata
        .iter()
        .map(|item| {
            let id = item.id.as_deref().and_then(|id| id.parse().ok());
            let mut value = item.as_ref().clone();
            value.unread = id
                .filter(|_| reading.generation == generation)
                .and_then(|id| reading.channels.get(&id))
                .cloned();
            value.badges = id
                .filter(|_| badges.version != 0 && badges.generation == generation)
                .and_then(|id| badges.channels.get(&id))
                .cloned();
            if let Some(old) = by_id.get(&value.id.as_deref().unwrap_or_default())
                && reusable
                && old.as_ref() == &value
            {
                return Arc::clone(old);
            }
            Arc::new(value)
        })
        .collect()
}

/// Every source baseline is marked seen before registration. Later changes remain pending,
/// so the list baseline cannot lose an intervening read mark or badge update.
pub(crate) fn subscribe(
    metadata: ChannelListSubscription,
    mut reading: watch::Receiver<Arc<ReadingSnapshot>>,
    mut badges: watch::Receiver<Arc<BadgesSnapshot>>,
) -> ChannelListSubscription {
    let baseline = Arc::clone(&reading.borrow_and_update());
    let badge_baseline = Arc::clone(&badges.borrow_and_update());
    let snapshot = overlay(
        &metadata.snapshot,
        &baseline,
        &badge_baseline,
        metadata.generation,
        &Vector::new(),
    );
    let version = metadata.version;
    let generation = metadata.generation;
    let mut publisher = Publisher::<_, _, ()>::new(version, generation, snapshot.clone());
    let receiver = publisher.subscribe(1);
    let changes = stream::unfold(
        (
            metadata.stream,
            reading,
            badges,
            metadata.snapshot,
            publisher,
            receiver,
        ),
        |(mut metadata, mut reading, mut badges, mut raw, mut publisher, mut receiver)| async move {
            let update = tokio::select! {
                biased;
                update = metadata.next() => Some(update?),
                result = reading.changed() => { result.ok()?; None },
                result = badges.changed() => { result.ok()?; None },
            };
            let reset = if let Some(update) = update {
                let reset = update.generation != publisher.generation
                    || update
                        .diffs
                        .iter()
                        .any(|diff| matches!(diff, eyeball_im::VectorDiff::Reset { .. }));
                publisher.generation = update.generation;
                for diff in update.diffs {
                    diff.apply(&mut raw);
                }
                reset
            } else {
                false
            };
            let current = Arc::clone(&reading.borrow_and_update());
            let badge_image = Arc::clone(&badges.borrow_and_update());
            let next = overlay(
                &raw,
                &current,
                &badge_image,
                publisher.generation,
                &publisher.items,
            );
            let version = publisher.version.checked_add(1)?;
            let reset = reset
                || !crate::channel_projection::unique_ids(
                    raw.iter().map(|item| item.id.as_deref()),
                );
            publisher.publish(version, publisher.generation, next, reset, |item| {
                item.id.clone()
            });
            let Event::Update {
                version,
                generation,
                diffs,
            } = receiver.next().await?
            else {
                return None;
            };
            Some((
                ChannelListUpdate {
                    version,
                    generation,
                    diffs,
                },
                (metadata, reading, badges, raw, publisher, receiver),
            ))
        },
    );
    ChannelListSubscription {
        version,
        generation,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}

#[cfg(test)]
mod tests;
