//! Auxiliary diff semantics and invalid-index cases for the plain-list proof.
//! The parent module owns the list fixture and the primary timeline scenarios.

use super::{Anchor, Arc, InvalidIndex, ListModel, MissingAnchor, Position, VectorDiff, item};

#[test]
fn anchor_is_resolved_after_the_whole_batch_not_during_temporary_absence() {
    let mut list = ListModel::from_snapshot(vec![item(10), item(20)]);
    let anchor = Anchor {
        item_id: 20,
        offset_px: 7,
    };

    list.apply_batch([
        VectorDiff::Remove { index: 1 },
        VectorDiff::Insert {
            index: 0,
            value: item(20),
        },
    ])
    .unwrap();

    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 0,
            offset_px: 7
        })
    );
}

#[test]
fn remaining_diff_variants_apply_in_order_and_keep_surviving_arcs() {
    let survivor = item(20);
    let mut list = ListModel::from_snapshot(vec![Arc::clone(&survivor)]);

    let batch = [
        VectorDiff::Append {
            values: [item(30), item(40)].into_iter().collect(),
        },
        VectorDiff::PushFront { value: item(10) },
        VectorDiff::PopFront,
        VectorDiff::PopBack,
        VectorDiff::Insert {
            index: 2,
            value: item(50),
        },
        VectorDiff::Remove { index: 1 },
        VectorDiff::Truncate { length: 1 },
    ];
    let expected_orders = [
        vec![20, 30, 40],
        vec![10, 20, 30, 40],
        vec![20, 30, 40],
        vec![20, 30],
        vec![20, 30, 50],
        vec![20, 50],
        vec![20],
    ];

    for (diff, expected) in batch.into_iter().zip(expected_orders) {
        list.apply_batch([diff]).unwrap();
        assert_eq!(list.ids(), expected);
        let actual = list.items.iter().find(|item| item.id == 20).unwrap();
        assert!(Arc::ptr_eq(actual, &survivor));
    }
}

#[test]
fn clear_empty_pops_and_oversized_truncation_match_vector_diff_semantics() {
    let mut list = ListModel::from_snapshot(vec![item(10)]);
    let anchor = Anchor {
        item_id: 10,
        offset_px: 4,
    };

    list.apply_batch([
        VectorDiff::Clear,
        VectorDiff::PopFront,
        VectorDiff::PopBack,
        VectorDiff::Truncate { length: 10 },
    ])
    .unwrap();

    assert_eq!(list.ids(), Vec::<u64>::new());
    assert_eq!(anchor.resolve(&list), Err(MissingAnchor(anchor)));
}

#[test]
fn invalid_indices_are_typed_and_do_not_mutate_the_rejected_diff() {
    let snapshot = vec![item(10)];
    let cases = [
        (
            VectorDiff::Insert {
                index: 2,
                value: item(20),
            },
            InvalidIndex::Insert { index: 2, len: 1 },
        ),
        (
            VectorDiff::Set {
                index: 1,
                value: item(20),
            },
            InvalidIndex::Set { index: 1, len: 1 },
        ),
        (
            VectorDiff::Remove { index: 1 },
            InvalidIndex::Remove { index: 1, len: 1 },
        ),
    ];

    for (diff, expected) in cases {
        let mut list = ListModel::from_snapshot(snapshot.clone());
        assert_eq!(list.apply_batch([diff]), Err(expected));
        assert_eq!(list.ids(), [10]);
        assert!(Arc::ptr_eq(&list.items[0], &snapshot[0]));
    }
}
