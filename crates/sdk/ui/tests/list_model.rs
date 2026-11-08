//! Framework-neutral proof of applying real `VectorDiff` batches to a plain list.
//! Scroll anchors are client state; subscription production and rendering are out of scope.

use std::sync::Arc;

use eyeball_im::VectorDiff;

#[derive(Debug, PartialEq, Eq)]
struct Item {
    id: u64,
    revision: u32,
}

fn item(id: u64) -> Arc<Item> {
    Arc::new(Item { id, revision: 0 })
}

/// Client-owned reading position, independent of list indices and allocations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    item_id: u64,
    offset_px: i32,
}

#[derive(Debug, PartialEq, Eq)]
struct Position {
    index: usize,
    offset_px: i32,
}

#[derive(Debug, PartialEq, Eq)]
struct MissingAnchor(Anchor);

impl Anchor {
    /// Resolves only after a complete batch; absence is explicit, never a fallback row.
    fn resolve(self, list: &ListModel) -> Result<Position, MissingAnchor> {
        list.items
            .iter()
            .position(|item| item.id == self.item_id)
            .map(|index| Position {
                index,
                offset_px: self.offset_px,
            })
            .ok_or(MissingAnchor(self))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InvalidIndex {
    Insert { index: usize, len: usize },
    Set { index: usize, len: usize },
    Remove { index: usize, len: usize },
}

struct ListModel {
    items: Vec<Arc<Item>>,
}

impl ListModel {
    fn from_snapshot(items: Vec<Arc<Item>>) -> Self {
        Self { items }
    }

    /// Applies diffs in delivery order, with each index relative to preceding diffs.
    ///
    /// Invalid indices return a typed error before that diff mutates the list;
    /// earlier valid diffs remain applied. Empty pops and oversized truncation are no-ops,
    /// matching `VectorDiff::apply`. Valid fixtures contain unique stable IDs.
    fn apply_batch(
        &mut self,
        batch: impl IntoIterator<Item = VectorDiff<Arc<Item>>>,
    ) -> Result<(), InvalidIndex> {
        for diff in batch {
            let len = self.items.len();
            match diff {
                VectorDiff::Append { values } => self.items.extend(values),
                VectorDiff::Clear => self.items.clear(),
                VectorDiff::PushFront { value } => self.items.insert(0, value),
                VectorDiff::PushBack { value } => self.items.push(value),
                VectorDiff::PopFront => {
                    if !self.items.is_empty() {
                        self.items.remove(0);
                    }
                }
                VectorDiff::PopBack => {
                    self.items.pop();
                }
                VectorDiff::Insert { index, value } => {
                    if index > len {
                        return Err(InvalidIndex::Insert { index, len });
                    }
                    self.items.insert(index, value);
                }
                VectorDiff::Set { index, value } => {
                    let slot = self
                        .items
                        .get_mut(index)
                        .ok_or(InvalidIndex::Set { index, len })?;
                    *slot = value;
                }
                VectorDiff::Remove { index } => {
                    if index >= len {
                        return Err(InvalidIndex::Remove { index, len });
                    }
                    self.items.remove(index);
                }
                VectorDiff::Truncate { length } => self.items.truncate(length),
                VectorDiff::Reset { values } => self.items = values.into_iter().collect(),
            }
        }
        Ok(())
    }

    fn ids(&self) -> Vec<u64> {
        self.items.iter().map(|item| item.id).collect()
    }
}

#[test]
fn snapshot_keeps_order_and_shared_item_identity() {
    let snapshot = vec![item(10), item(20), item(30)];
    let list = ListModel::from_snapshot(snapshot.clone());

    assert_eq!(list.ids(), [10, 20, 30]);
    for (actual, original) in list.items.iter().zip(&snapshot) {
        assert!(Arc::ptr_eq(actual, original));
    }
}

#[test]
fn push_back_appends_one_item_without_replacing_existing_arcs() {
    let snapshot = vec![item(10), item(20)];
    let mut list = ListModel::from_snapshot(snapshot.clone());
    let newest = item(30);

    list.apply_batch([VectorDiff::PushBack {
        value: Arc::clone(&newest),
    }])
    .unwrap();

    assert_eq!(list.ids(), [10, 20, 30]);
    assert!(Arc::ptr_eq(&list.items[2], &newest));
    for (actual, original) in list.items.iter().zip(&snapshot) {
        assert!(Arc::ptr_eq(actual, original));
    }
}

#[test]
fn set_replaces_only_the_edited_arc_while_preserving_id_and_anchor() {
    let snapshot = vec![item(10), item(20), item(30)];
    let mut list = ListModel::from_snapshot(snapshot.clone());
    let anchor = Anchor {
        item_id: 20,
        offset_px: -17,
    };
    let edited = Arc::new(Item {
        id: 20,
        revision: 1,
    });

    list.apply_batch([VectorDiff::Set {
        index: 1,
        value: Arc::clone(&edited),
    }])
    .unwrap();

    assert_eq!(list.ids(), [10, 20, 30]);
    assert_eq!(list.items[1].revision, 1);
    assert!(Arc::ptr_eq(&list.items[1], &edited));
    assert!(!Arc::ptr_eq(&list.items[1], &snapshot[1]));
    assert!(Arc::ptr_eq(&list.items[0], &snapshot[0]));
    assert!(Arc::ptr_eq(&list.items[2], &snapshot[2]));
    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 1,
            offset_px: -17
        })
    );
}

#[test]
fn multiple_inserts_at_zero_in_one_batch_keep_chronology_and_anchor() {
    let snapshot = vec![item(30), item(40)];
    let mut list = ListModel::from_snapshot(snapshot.clone());
    let anchor = Anchor {
        item_id: 40,
        offset_px: 23,
    };

    // A chronological page is visited newest-first when every insertion targets zero.
    list.apply_batch([
        VectorDiff::Insert {
            index: 0,
            value: item(20),
        },
        VectorDiff::Insert {
            index: 0,
            value: item(10),
        },
    ])
    .unwrap();

    assert_eq!(list.ids(), [10, 20, 30, 40]);
    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 3,
            offset_px: 23
        })
    );
    assert!(Arc::ptr_eq(&list.items[2], &snapshot[0]));
    assert!(Arc::ptr_eq(&list.items[3], &snapshot[1]));
}

#[test]
fn repeated_prepend_batches_keep_the_original_anchor_and_arcs() {
    let snapshot = vec![item(50), item(60)];
    let mut list = ListModel::from_snapshot(snapshot.clone());
    let anchor = Anchor {
        item_id: 50,
        offset_px: -9,
    };

    list.apply_batch([
        VectorDiff::Insert {
            index: 0,
            value: item(40),
        },
        VectorDiff::Insert {
            index: 0,
            value: item(30),
        },
    ])
    .unwrap();
    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 2,
            offset_px: -9
        })
    );

    list.apply_batch([
        VectorDiff::Insert {
            index: 0,
            value: item(20),
        },
        VectorDiff::Insert {
            index: 0,
            value: item(10),
        },
    ])
    .unwrap();

    assert_eq!(list.ids(), [10, 20, 30, 40, 50, 60]);
    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 4,
            offset_px: -9
        })
    );
    assert!(Arc::ptr_eq(&list.items[4], &snapshot[0]));
    assert!(Arc::ptr_eq(&list.items[5], &snapshot[1]));
}

#[test]
fn reset_restores_anchor_by_id_with_new_arc_and_reordered_index() {
    let original = item(20);
    let mut list = ListModel::from_snapshot(vec![item(10), Arc::clone(&original), item(30)]);
    let anchor = Anchor {
        item_id: 20,
        offset_px: 31,
    };
    let replacement = Arc::new(Item {
        id: 20,
        revision: 2,
    });

    list.apply_batch([VectorDiff::Reset {
        values: [Arc::clone(&replacement), item(30), item(10)]
            .into_iter()
            .collect(),
    }])
    .unwrap();

    assert_eq!(list.ids(), [20, 30, 10]);
    assert!(Arc::ptr_eq(&list.items[0], &replacement));
    assert!(!Arc::ptr_eq(&list.items[0], &original));
    assert_eq!(
        anchor.resolve(&list),
        Ok(Position {
            index: 0,
            offset_px: 31
        })
    );
}

#[test]
fn reset_reports_missing_anchor_instead_of_selecting_the_old_index() {
    let mut list = ListModel::from_snapshot(vec![item(10), item(20)]);
    let anchor = Anchor {
        item_id: 20,
        offset_px: -12,
    };

    list.apply_batch([VectorDiff::Reset {
        values: [item(30), item(40)].into_iter().collect(),
    }])
    .unwrap();

    assert_eq!(list.ids(), [30, 40]);
    assert_eq!(anchor.resolve(&list), Err(MissingAnchor(anchor)));
}

#[path = "list_model/edge_cases.rs"]
mod edge_cases;
