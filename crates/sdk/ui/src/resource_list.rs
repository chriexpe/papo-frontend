//! Ordered list projections from immutable account images. Source ordering and admission belong to the SDK.

use eyeball_im::VectorDiff;
use futures_util::{Stream, StreamExt, stream};
use imbl::Vector;
use papo_sdk::{Generation, ResourceSnapshot, Uuid};
use std::{collections::BTreeSet, pin::Pin, sync::Arc};
use tokio::sync::watch;

/// An account resource projection with stable item identity and bounded source coalescing.
pub struct PeopleListSubscription<T: Clone> {
    /// Resource publication represented by the atomic baseline.
    pub version: u64,
    /// Connection owning the resource baseline.
    pub generation: Generation,
    /// Initial view-ready items. Unchanged items keep their Arc in subsequent updates.
    pub snapshot: Vector<Arc<T>>,
    /// Sequential edits from the last observed image. A generation change resets the baseline.
    pub stream: Pin<Box<dyn Stream<Item = PeopleListUpdate<T>> + Send>>,
}
/// One complete list transition.
pub struct PeopleListUpdate<T: Clone> {
    /// Resource version after applying the entire batch.
    pub version: u64,
    /// Connection epoch owning these items.
    pub generation: Generation,
    /// Edits to apply in order.
    pub diffs: Vec<VectorDiff<Arc<T>>>,
}

#[derive(Clone, Copy)]
pub(crate) enum List {
    Members,
    Blocks,
    Roles,
    Profiles,
    Channels,
}

impl List {
    fn order(self, image: &ResourceSnapshot) -> &Vector<Uuid> {
        match self {
            Self::Members => image.members.order(),
            Self::Blocks => image.blocks.order(),
            Self::Profiles => image.profiles.order(),
            Self::Roles => image.roles.order(),
            Self::Channels => image.channels.order(),
        }
    }

    fn ids(self, image: &ResourceSnapshot) -> impl Iterator<Item = Uuid> {
        self.order(image).iter().copied()
    }

    fn changed(self, current: &ResourceSnapshot, old: &ResourceSnapshot) -> BTreeSet<Uuid> {
        fn label(role: &papo_sdk::RoleRecord) -> (Option<&str>, Option<&str>) {
            (role.name.as_deref(), role.color.as_deref())
        }
        match self {
            Self::Channels => return current.channels.diff_keys(&old.channels).collect(),
            Self::Roles => return current.roles.diff_keys(&old.roles).collect(),
            Self::Members | Self::Profiles | Self::Blocks => {}
        }
        let mut changed: BTreeSet<_> = current.users.diff_keys(&old.users).collect();
        if matches!(self, Self::Profiles) {
            changed.extend(current.profiles.diff_keys(&old.profiles));
        }
        let roles: BTreeSet<_> = current
            .roles
            .diff_keys(&old.roles)
            .filter(|&id| current.roles.get(id).map(label) != old.roles.get(id).map(label))
            .collect();
        if !roles.is_empty() {
            changed.extend(self.ids(current).filter(|&id| {
                current
                    .users
                    .get(id)
                    .and_then(|user| user.roles.as_ref())
                    .is_some_and(|assigned| assigned.iter().any(|role| roles.contains(role)))
            }));
        }
        changed
    }
}

pub(crate) fn subscribe<T>(
    mut source: watch::Receiver<Arc<ResourceSnapshot>>,
    list: List,
    project: fn(&ResourceSnapshot, Uuid) -> T,
) -> PeopleListSubscription<T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let baseline = Arc::clone(&source.borrow_and_update());
    let items = list
        .ids(&baseline)
        .map(|id| Arc::new(project(&baseline, id)))
        .collect::<Vector<_>>();
    let version = baseline.version;
    let generation = baseline.generation;
    let snapshot = items.clone();
    let changes = stream::unfold(
        (source, items, baseline),
        move |(mut source, mut items, baseline)| async move {
            source.changed().await.ok()?;
            let current = Arc::clone(&source.borrow_and_update());
            let diffs = reconcile(list, &baseline, &current, &mut items, project);
            let update = PeopleListUpdate {
                version: current.version,
                generation: current.generation,
                diffs,
            };
            Some((update, (source, items, current)))
        },
    );
    PeopleListSubscription {
        version,
        generation,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}

fn reconcile<T: Clone + PartialEq>(
    list: List,
    old: &ResourceSnapshot,
    current: &ResourceSnapshot,
    items: &mut Vector<Arc<T>>,
    project: impl Fn(&ResourceSnapshot, Uuid) -> T,
) -> Vec<VectorDiff<Arc<T>>> {
    if old.generation != current.generation {
        *items = list
            .ids(current)
            .map(|id| Arc::new(project(current, id)))
            .collect();
        return vec![VectorDiff::Reset {
            values: items.clone(),
        }];
    }
    let mut changed = list.changed(current, old);
    let mut diffs = Vec::new();
    let before = list.order(old);
    let after = list.order(current);
    if !before.ptr_eq(after) && before != after {
        let mut order = before.clone();
        let mut removed = std::collections::BTreeMap::new();
        for edit in order_edits(before, after) {
            let mut inserted = |id, removed: &mut std::collections::BTreeMap<_, _>| {
                removed.remove(&id).unwrap_or_else(|| {
                    changed.remove(&id);
                    Arc::new(project(current, id))
                })
            };
            let diff = match edit {
                VectorDiff::Remove { index } => {
                    removed.insert(order[index], Arc::clone(&items[index]));
                    VectorDiff::Remove { index }
                }
                VectorDiff::Insert { index, value } => VectorDiff::Insert {
                    index,
                    value: inserted(value, &mut removed),
                },
                VectorDiff::PushBack { value } => VectorDiff::PushBack {
                    value: inserted(value, &mut removed),
                },
                VectorDiff::Reset { .. } => {
                    let previous: std::collections::BTreeMap<_, _> =
                        order.iter().copied().zip(items.iter()).collect();
                    let values = list
                        .ids(current)
                        .map(|id| {
                            previous.get(&id).map_or_else(
                                || Arc::new(project(current, id)),
                                |value| Arc::clone(value),
                            )
                        })
                        .collect();
                    VectorDiff::Reset { values }
                }
                _ => unreachable!("identity-only list diff produces only structural edits"),
            };
            edit.apply(&mut order);
            diff.clone().apply(items);
            diffs.push(diff);
        }
    }
    if !changed.is_empty() {
        for (index, id) in list.ids(current).enumerate() {
            if changed.contains(&id) {
                let next = project(current, id);
                if next != *items[index] {
                    let value = Arc::new(next);
                    items.set(index, Arc::clone(&value));
                    diffs.push(VectorDiff::Set { index, value });
                }
            }
        }
    }
    diffs
}

/// A single join or departure needs only a scan of borrowed IDs and one edit.
fn order_edits(before: &Vector<Uuid>, after: &Vector<Uuid>) -> Vec<VectorDiff<Uuid>> {
    if before.len().abs_diff(after.len()) == 1 {
        let inserting = after.len() > before.len();
        let (long, short) = if inserting {
            (after, before)
        } else {
            (before, after)
        };
        let index = long
            .iter()
            .zip(short)
            .position(|(a, b)| a != b)
            .unwrap_or(short.len());
        if long.iter().skip(index + 1).eq(short.iter().skip(index)) {
            let edit = if !inserting {
                VectorDiff::Remove { index }
            } else if index == short.len() {
                VectorDiff::PushBack { value: long[index] }
            } else {
                VectorDiff::Insert {
                    index,
                    value: long[index],
                }
            };
            return vec![edit];
        }
    }
    crate::list_diff::between(before, after, |id| *id)
}

#[cfg(test)]
mod tests;
