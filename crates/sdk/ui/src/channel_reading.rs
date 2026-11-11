//! Read-state overlay for channel metadata; canonical read policy stays in the SDK.

use futures_util::{
    StreamExt,
    future::{self, Either},
    stream,
};
use imbl::Vector;
use papo_sdk::ReadingSnapshot;
use std::sync::Arc;
use tokio::sync::watch;

use crate::{
    channel_list::{ChannelListItem, ChannelListSubscription, ChannelListUpdate},
    list_subscription::{Event, Publisher},
};

fn overlay(
    metadata: &Vector<Arc<ChannelListItem>>,
    reading: &ReadingSnapshot,
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
            let unread = if reading.generation == generation {
                item.id
                    .as_deref()
                    .and_then(|id| id.parse().ok())
                    .and_then(|id| reading.channels.get(&id))
                    .cloned()
            } else {
                None
            };
            let mut value = item.as_ref().clone();
            value.unread = unread;
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

/// Both source baselines are marked seen before registration. Updates after either capture
/// remain pending in its source stream, so the list baseline cannot lose an intervening mark.
pub(crate) fn subscribe(
    metadata: ChannelListSubscription,
    mut reading: watch::Receiver<Arc<ReadingSnapshot>>,
) -> ChannelListSubscription {
    let baseline = Arc::clone(&reading.borrow_and_update());
    let snapshot = overlay(
        &metadata.snapshot,
        &baseline,
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
            metadata.snapshot,
            publisher,
            receiver,
        ),
        |(mut metadata, mut reading, mut raw, mut publisher, mut receiver)| async move {
            let wake = {
                match future::select(Box::pin(metadata.next()), Box::pin(reading.changed())).await {
                    Either::Left((update, pending)) => {
                        drop(pending);
                        Either::Left(update)
                    }
                    Either::Right((result, pending)) => {
                        drop(pending);
                        Either::Right(result)
                    }
                }
            };
            let reset = match wake {
                Either::Left(Some(update)) => {
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
                }
                Either::Left(None) | Either::Right(Err(_)) => return None,
                Either::Right(Ok(())) => false,
            };
            let current = Arc::clone(&reading.borrow_and_update());
            let next = overlay(&raw, &current, publisher.generation, &publisher.items);
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
                (metadata, reading, raw, publisher, receiver),
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
