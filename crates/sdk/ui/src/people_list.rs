//! Pull-driven people list diffs from account resource snapshots; no network, roster fetching or platform UI.

use eyeball_im::VectorDiff;
use futures_util::{Stream, StreamExt, stream};
use imbl::Vector;
use papo_sdk::{Generation, ResourceSnapshot};
use std::{collections::BTreeMap, pin::Pin, sync::Arc};
use tokio::sync::watch;

/// An account resource projection with stable item identity and bounded source coalescing.
pub struct PeopleListSubscription<T: Clone> {
    /// Resource publication represented by the atomic baseline.
    pub version: u64,
    /// Connection owning the resource baseline.
    pub generation: Generation,
    /// Initial view-ready items. Unchanged items keep their Arc in subsequent updates.
    pub snapshot: Vector<Arc<T>>,
    /// Sequential batches; skipped source publications or new generations produce a Reset.
    pub stream: Pin<Box<dyn Stream<Item = PeopleListUpdate<T>> + Send>>,
}
/// One complete people-list transition.
pub struct PeopleListUpdate<T: Clone> {
    /// Resource version after applying the entire batch.
    pub version: u64,
    /// Connection epoch owning these items.
    pub generation: Generation,
    /// Edits to apply in order.
    pub diffs: Vec<VectorDiff<Arc<T>>>,
}

pub(crate) trait Identified {
    fn id(&self) -> &str;
}

pub(crate) fn subscribe<T, F>(
    mut source: watch::Receiver<Arc<ResourceSnapshot>>,
    project: F,
    relevant: fn(&ResourceSnapshot, &ResourceSnapshot) -> bool,
) -> PeopleListSubscription<T>
where
    T: Identified + Clone + PartialEq + Send + Sync + 'static,
    F: Fn(&ResourceSnapshot) -> Vec<T> + Send + 'static,
{
    let baseline = Arc::clone(&source.borrow_and_update());
    let items = reuse(project(&baseline), &Vector::new());
    let version = baseline.version;
    let generation = baseline.generation;
    let snapshot = items.clone();
    let changes = stream::unfold(
        (source, items, baseline, project),
        move |(mut source, previous, baseline, project)| async move {
            source.changed().await.ok()?;
            let current = Arc::clone(&source.borrow_and_update());
            let same_generation = baseline.generation == current.generation;
            let changed = !same_generation || relevant(&baseline, &current);
            let items = if changed {
                reuse(project(&current), &previous)
            } else {
                previous.clone()
            };
            let diffs = if current.generation != baseline.generation
                || baseline.version.checked_add(1) != Some(current.version)
            {
                vec![VectorDiff::Reset {
                    values: items.clone(),
                }]
            } else if !changed {
                Vec::new()
            } else {
                crate::list_diff::between(&previous, &items, |item| item.id().to_owned())
            };
            let update = PeopleListUpdate {
                version: current.version,
                generation: current.generation,
                diffs,
            };
            Some((update, (source, items, current, project)))
        },
    );
    PeopleListSubscription {
        version,
        generation,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}

fn reuse<T: Identified + PartialEq>(items: Vec<T>, previous: &Vector<Arc<T>>) -> Vector<Arc<T>> {
    let existing: BTreeMap<_, _> = previous.iter().map(|item| (item.id(), item)).collect();
    items
        .into_iter()
        .map(|item| {
            if let Some(old) = existing.get(item.id())
                && old.as_ref() == &item
            {
                return Arc::clone(old);
            }
            Arc::new(item)
        })
        .collect()
}

#[cfg(test)]
mod tests;
