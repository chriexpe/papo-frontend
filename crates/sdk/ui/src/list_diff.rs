//! Identity-aware list edits; source versions and subscription lifetimes belong to projections.

use eyeball_im::VectorDiff;
use imbl::Vector;

/// Produces sequential edits without replacing unchanged identities.
/// Both lists must have unique identities, as guaranteed by the SDK registry.
pub(crate) fn between<T: Clone + PartialEq, K: Eq>(
    previous: &Vector<T>,
    next: &Vector<T>,
    identity: impl Fn(&T) -> K,
) -> Vec<VectorDiff<T>> {
    let mut current = previous.clone();
    let mut diffs = Vec::new();
    for index in (0..current.len()).rev() {
        if !next
            .iter()
            .any(|item| identity(item) == identity(&current[index]))
        {
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
        if let Some(old_index) = current
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
