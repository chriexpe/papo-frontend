//! Pull-driven channel projection; SDK commits resources and owns watch publication/closure.

use std::{
    collections::{BTreeMap, BTreeSet},
    pin::Pin,
    sync::Arc,
};

use eyeball_im::VectorDiff;
use futures_util::{Stream, StreamExt, stream};
use imbl::Vector;
use papo_sdk::{Channel, Generation, ResourceSnapshot};
use tokio::sync::watch;

use crate::channel_list::{ChannelListItem, ChannelListSubscription, ChannelListUpdate};
use crate::list_diff;

/// Reads public SDK snapshots; the generic epoch permits deterministic isolated watch tests.
trait Source: Send + Sync + 'static {
    type Generation: Copy + Eq + Send + 'static;

    fn version(&self) -> u64;
    fn generation(&self) -> Self::Generation;
    fn channels(&self) -> &[Channel];
}

impl Source for ResourceSnapshot {
    type Generation = Generation;

    fn version(&self) -> u64 {
        self.version
    }

    fn generation(&self) -> Generation {
        self.generation
    }

    fn channels(&self) -> &[Channel] {
        &self.channels
    }
}

/// One resource publication, independent of the concrete connection epoch type.
struct Update<G> {
    version: u64,
    generation: G,
    diffs: Vec<VectorDiff<Arc<ChannelListItem>>>,
}

/// Atomically captured baseline with its independent capacity-one source receiver.
struct Subscription<G> {
    version: u64,
    generation: G,
    snapshot: Vector<Arc<ChannelListItem>>,
    stream: Pin<Box<dyn Stream<Item = Update<G>> + Send>>,
}

/// Converts the internal projection into the public SDK-generation-stamped surface.
pub(crate) fn subscribe(source: watch::Receiver<Arc<ResourceSnapshot>>) -> ChannelListSubscription {
    let subscription = subscribe_source(source);
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

/// Marks exactly the cloned source publication seen before projecting outside the watch lock.
fn subscribe_source<S: Source>(mut source: watch::Receiver<Arc<S>>) -> Subscription<S::Generation> {
    let baseline = Arc::clone(&source.borrow_and_update());
    let version = baseline.version();
    let generation = baseline.generation();
    let snapshot = project(baseline.channels(), &Vector::new());
    let changes = stream::unfold(
        (source, version, generation, snapshot.clone()),
        |(mut source, version, generation, previous)| async move {
            source.changed().await.ok()?;
            let current = Arc::clone(&source.borrow_and_update());
            let next = project(current.channels(), &previous);
            let consecutive = version.checked_add(1) == Some(current.version());
            let same_generation = generation == current.generation();
            let unambiguous = unique_ids(previous.iter().map(|item| item.id.as_deref()))
                && unique_ids(next.iter().map(|item| item.id.as_deref()));
            let diffs = if consecutive && same_generation && unambiguous {
                list_diff::between(&previous, &next, |item| item.id.clone())
            } else {
                vec![VectorDiff::Reset {
                    values: next.clone(),
                }]
            };
            let update = Update {
                version: current.version(),
                generation: current.generation(),
                diffs,
            };
            Some((
                update,
                (source, current.version(), current.generation(), next),
            ))
        },
    );
    Subscription {
        version,
        generation,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}

/// Projects only backend metadata and reuses unchanged items with unambiguous backend IDs.
fn project(
    channels: &[Channel],
    previous: &Vector<Arc<ChannelListItem>>,
) -> Vector<Arc<ChannelListItem>> {
    let reusable = unique_ids(previous.iter().map(|item| item.id.as_deref()))
        && unique_ids(channels.iter().map(|channel| channel.id.as_deref()));
    let by_id: BTreeMap<_, _> = previous
        .iter()
        .filter_map(|item| item.id.as_deref().map(|id| (id, item)))
        .collect();
    channels
        .iter()
        .map(|channel| {
            let value = ChannelListItem {
                id: channel.id.clone(),
                name: channel.name.clone(),
                kind: channel.r#type.clone(),
                position: channel.position,
                topic: channel.topic.clone(),
                created_at: channel.created_at.clone(),
            };
            if reusable
                && let Some(old) = value.id.as_deref().and_then(|id| by_id.get(id))
                && old.as_ref() == &value
            {
                return Arc::clone(old);
            }
            Arc::new(value)
        })
        .collect()
}

/// Missing, empty and duplicated IDs do not establish identity; all rows still survive projection.
fn unique_ids<'a>(mut ids: impl Iterator<Item = Option<&'a str>>) -> bool {
    let mut seen = BTreeSet::new();
    ids.all(|id| id.is_some_and(|id| !id.is_empty() && seen.insert(id)))
}

#[cfg(test)]
mod tests;
