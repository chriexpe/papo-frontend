//! Identity-aware list edits; source versions and subscription lifetimes belong to projections.

use std::collections::{BTreeMap, BTreeSet};

use eyeball_im::VectorDiff;
use imbl::Vector;

/// Produces sequential edits without replacing unchanged identities.
/// Both lists must have unique identities, as guaranteed by the SDK registry.
pub(crate) fn between<T: Clone + PartialEq, K: Ord>(
    previous: &Vector<T>,
    next: &Vector<T>,
    identity: impl Fn(&T) -> K,
) -> Vec<VectorDiff<T>> {
    let positions: BTreeMap<_, _> = previous
        .iter()
        .enumerate()
        .map(|(index, item)| (identity(item), index))
        .collect();
    let identities: BTreeSet<_> = next.iter().map(&identity).collect();
    let mut last = None;
    let reordered = next
        .iter()
        .filter_map(|item| positions.get(&identity(item)))
        .any(|&index| {
            let moved = last.is_some_and(|last| last > index);
            last = Some(index);
            moved
        });
    // Bound general move searches; common append/prepend/edit/delete paths never search.
    if reordered && previous.len().max(next.len()) > 64 {
        return vec![VectorDiff::Reset {
            values: next.clone(),
        }];
    }
    let mut current = previous.clone();
    let mut diffs = Vec::new();
    for index in (0..current.len()).rev() {
        if !identities.contains(&identity(&current[index])) {
            current.remove(index);
            diffs.push(VectorDiff::Remove { index });
        }
    }
    for (index, value) in next.iter().enumerate() {
        if current
            .get(index)
            .is_some_and(|item| identity(item) == identity(value))
        {
            if current[index] != *value {
                current.set(index, value.clone());
                diffs.push(VectorDiff::Set {
                    index,
                    value: value.clone(),
                });
            }
            continue;
        }
        if reordered
            && let Some(old_index) = current
                .iter()
                .position(|item| identity(item) == identity(value))
        {
            current.remove(old_index);
            diffs.push(VectorDiff::Remove { index: old_index });
        }
        if index == current.len() {
            current.push_back(value.clone());
            diffs.push(VectorDiff::PushBack {
                value: value.clone(),
            });
        } else {
            current.insert(index, value.clone());
            diffs.push(VectorDiff::Insert {
                index,
                value: value.clone(),
            });
        }
    }
    diffs
}

#[cfg(test)]
mod tests {
    //! Exercises list identity without domain records, network or runtime state.

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Item {
        id: u8,
        revision: u8,
    }

    fn items(ids: &[u8]) -> Vector<Item> {
        ids.iter().map(|&id| Item { id, revision: 0 }).collect()
    }

    #[test]
    fn front_insertion_does_not_replace_existing_items() {
        let diffs = between(&items(&[2, 3]), &items(&[1, 2, 3]), |item| item.id);
        assert_eq!(
            diffs,
            vec![VectorDiff::Insert {
                index: 0,
                value: Item { id: 1, revision: 0 }
            }]
        );
    }

    #[test]
    fn metadata_update_sets_only_the_same_identity() {
        let mut next = items(&[1, 2]);
        next.set(1, Item { id: 2, revision: 1 });
        let diffs = between(&items(&[1, 2]), &next, |item| item.id);
        assert_eq!(
            diffs,
            vec![VectorDiff::Set {
                index: 1,
                value: Item { id: 2, revision: 1 }
            }]
        );
    }

    #[test]
    fn reorder_keeps_unmoved_identity_untouched() {
        let diffs = between(&items(&[1, 2, 3]), &items(&[3, 1, 2]), |item| item.id);
        assert_eq!(
            diffs,
            vec![
                VectorDiff::Remove { index: 2 },
                VectorDiff::Insert {
                    index: 0,
                    value: Item { id: 3, revision: 0 }
                }
            ]
        );
    }

    #[test]
    fn every_small_unique_list_transition_reconstructs_the_target() {
        let lists: Vec<_> = (0..4)
            .flat_map(|a| (0..4).flat_map(move |b| (0..4).map(move |c| [a, b, c])))
            .filter(|ids| ids[0] != ids[1] && ids[0] != ids[2] && ids[1] != ids[2])
            .flat_map(|ids| (0..=3).map(move |length| items(&ids[..length])))
            .collect();
        for previous in &lists {
            for next in &lists {
                let mut actual = previous.clone();
                for diff in between(previous, next, |item| item.id) {
                    diff.apply(&mut actual);
                }
                assert_eq!(&actual, next);
            }
        }
    }
}

#[cfg(test)]
mod scaling_tests {
    //! Deterministic identity-work budgets supplement replay properties without timing assertions.

    use super::*;
    use std::cell::Cell;

    #[test]
    fn large_append_prepend_edit_delete_and_reorder_have_bounded_identity_work() {
        for count in [100, 1_000, 10_000] {
            let previous: Vector<_> = (0..count).map(|id| (id, 0)).collect();
            let mut append = previous.clone();
            append.push_back((count, 0));
            let prepend = (count..count + 100)
                .map(|id| (id, 0))
                .chain(previous.iter().copied())
                .collect();
            let mut edit = previous.clone();
            edit.set(count / 2, (count / 2, 1));
            let mut delete = previous.clone();
            delete.remove(count / 2);
            let reorder = previous.iter().rev().copied().collect();
            for next in [append, prepend, edit, delete, reorder] {
                let calls = Cell::new(0);
                let diffs = between(&previous, &next, |item| {
                    calls.set(calls.get() + 1);
                    item.0
                });
                assert!(calls.get() <= 8 * (previous.len() + next.len()));
                let mut applied = previous.clone();
                for diff in diffs {
                    diff.apply(&mut applied);
                }
                assert_eq!(applied, next);
            }
        }
    }
}
