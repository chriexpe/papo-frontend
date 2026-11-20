//! Read and badge overlays for channel metadata; canonical evidence and policy stay in the SDK.

use futures_util::{
    StreamExt,
    stream::{self, FusedStream},
};
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
    previous_metadata: &Vector<Arc<ChannelListItem>>,
    previous: &Vector<Arc<ChannelListItem>>,
) -> Vector<Arc<ChannelListItem>> {
    let by_id: std::collections::BTreeMap<_, _> = previous_metadata
        .iter()
        .zip(previous)
        .map(|(raw, row)| (raw.id, (raw, row)))
        .collect();
    metadata
        .iter()
        .map(|item| {
            #[cfg(test)]
            tests::VISITS.set(tests::VISITS.get() + 1);
            let unread = (reading.generation == generation)
                .then(|| reading.channels.get(&item.id))
                .flatten();
            let badge = (badges.version != 0 && badges.generation == generation)
                .then(|| badges.channels.get(&item.id))
                .flatten();
            if let Some(&(old_metadata, old_row)) = by_id.get(&item.id)
                && (Arc::ptr_eq(old_metadata, item) || old_metadata == item)
                && old_row.unread.as_ref() == unread
                && old_row.badges.as_ref() == badge
            {
                return Arc::clone(old_row);
            }
            if unread.is_none() && badge.is_none() {
                return Arc::clone(item);
            }
            #[cfg(test)]
            tests::ROW_CLONES.set(tests::ROW_CLONES.get() + 1);
            let mut value = item.as_ref().clone();
            value.unread = unread.cloned();
            value.badges = badge.cloned();
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
        &Vector::new(),
    );
    let version = metadata.version;
    let generation = metadata.generation;
    let mut publisher = Publisher::<_, _, ()>::new(version, generation, snapshot.clone());
    let receiver = publisher.subscribe(1);
    let changes = stream::unfold(
        (
            metadata.stream.fuse(),
            reading,
            badges,
            metadata.snapshot,
            publisher,
            receiver,
        ),
        |(mut metadata, mut reading, mut badges, mut raw, mut publisher, mut receiver)| async move {
            loop {
                if metadata.is_terminated() {
                    return None;
                }
                let update = tokio::select! {
                    biased;
                    update = metadata.next() => {
                        // The final metadata image may have been silent. Drain any pending
                        // overlay once before ending the account subscription.
                        if update.is_none() && !reading.borrow().has_changed() && !badges.borrow().has_changed() {
                            return None;
                        }
                        update
                    },
                    result = reading.changed() => { result.ok()?; None },
                    result = badges.changed() => { result.ok()?; None },
                };
                let previous_metadata = raw.clone();
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
                let empty = Vector::new();
                let next = overlay(
                    &raw,
                    &current,
                    &badge_image,
                    publisher.generation,
                    &previous_metadata,
                    if reset { &empty } else { &publisher.items },
                );
                if !reset && next == publisher.items {
                    continue;
                }
                let version = publisher.version.checked_add(1)?;
                publisher.publish(version, publisher.generation, next, reset, |item| item.id);
                let Event::Update {
                    version,
                    generation,
                    diffs,
                } = receiver.next().await?
                else {
                    return None;
                };
                return Some((
                    ChannelListUpdate {
                        version,
                        generation,
                        diffs,
                    },
                    (metadata, reading, badges, raw, publisher, receiver),
                ));
            }
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
