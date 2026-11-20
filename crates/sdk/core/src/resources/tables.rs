//! Immutable UUID-keyed entity tables and source order. Admission owns uniqueness.

use crate::Uuid;
use imbl::{OrdMap, Vector, ordmap::DiffItem};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct Shared<T>(Arc<T>);

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}
impl<T: Eq> Eq for Shared<T> {}

/// Persistent identity map. Cloning a snapshot retains its unchanged entity allocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityTable<T: Clone + Eq>(OrdMap<Uuid, Shared<T>>);

impl<T: Clone + Eq> Default for EntityTable<T> {
    fn default() -> Self {
        Self(OrdMap::new())
    }
}

impl<T: Clone + Eq> EntityTable<T> {
    /// Returns the record with this identity.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&T> {
        self.0.get(&id).map(|value| value.0.as_ref())
    }

    /// Retains the immutable entity independently of its snapshot.
    #[must_use]
    pub fn get_arc(&self, id: Uuid) -> Option<Arc<T>> {
        self.0.get(&id).map(|value| Arc::clone(&value.0))
    }

    /// Identities added, changed or removed since any earlier snapshot.
    pub fn diff_keys<'a>(&'a self, previous: &'a Self) -> impl Iterator<Item = Uuid> + 'a {
        previous.0.diff(&self.0).map(|change| match change {
            DiffItem::Add(id, _)
            | DiffItem::Remove(id, _)
            | DiffItem::Update { new: (id, _), .. } => *id,
        })
    }

    pub(crate) fn set(&mut self, id: Uuid, value: T) {
        if self.get(id) != Some(&value) {
            self.0.insert(id, Shared(Arc::new(value)));
        }
    }

    pub(crate) fn remove(&mut self, id: Uuid) {
        self.0.remove(&id);
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.0.keys().copied()
    }
}

/// Backend-ordered identity references. Positions are never persisted in a second index.
#[derive(Clone, Debug, Default, Eq)]
pub struct ResourceIds {
    order: Vector<Uuid>,
    present: imbl::OrdSet<Uuid>,
}

impl PartialEq for ResourceIds {
    fn eq(&self, other: &Self) -> bool {
        self.same_order(other)
    }
}

impl ResourceIds {
    /// Iterates in backend order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Uuid> + '_ {
        self.order.iter().copied()
    }

    /// Whether this relationship is present.
    #[must_use]
    pub fn contains(&self, id: Uuid) -> bool {
        self.present.contains(&id)
    }

    /// Persistent source order, available for reconciling retained versions without copying IDs.
    #[must_use]
    pub fn order(&self) -> &Vector<Uuid> {
        &self.order
    }

    /// Number of relationships.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether there are no relationships.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Whether two versions preserve the same identity sequence.
    #[must_use]
    pub fn same_order(&self, other: &Self) -> bool {
        self.order.ptr_eq(&other.order) || self.order == other.order
    }

    pub(crate) fn replace(&mut self, ids: impl Iterator<Item = Uuid>) {
        let order: Vector<_> = ids.collect();
        if order != self.order {
            self.present = order.iter().copied().collect();
            self.order = order;
        }
    }

    pub(crate) fn insert(&mut self, index: usize, id: Uuid) {
        if self.present.insert(id).is_none() {
            self.order.insert(index, id);
        }
    }

    pub(crate) fn remove(&mut self, id: Uuid) {
        if self.present.remove(&id).is_some() {
            remove_from_order(&mut self.order, id);
        }
    }
}

/// Persistent records with a separate backend-defined order.
#[derive(Clone, Debug, Eq)]
pub struct ResourceTable<T: Clone + Eq> {
    order: Vector<Uuid>,
    rows: EntityTable<T>,
}

impl<T: Clone + Eq> PartialEq for ResourceTable<T> {
    fn eq(&self, other: &Self) -> bool {
        self.same_order(other) && self.rows == other.rows
    }
}

fn remove_from_order(order: &mut Vector<Uuid>, id: Uuid) {
    let index = order.iter().position(|value| *value == id);
    if let Some(index) = index {
        order.remove(index);
    }
}

impl<T: Clone + Eq> Default for ResourceTable<T> {
    fn default() -> Self {
        Self {
            order: Vector::new(),
            rows: EntityTable::default(),
        }
    }
}

impl<T: Clone + Eq> ResourceTable<T> {
    /// Builds a table from admitted rows in response order.
    pub(crate) fn from_rows(rows: impl IntoIterator<Item = (Uuid, T)>) -> Self {
        let mut table = Self::default();
        for (id, row) in rows {
            table.set(id, row);
        }
        table
    }
    /// Records in backend order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.ids().filter_map(|id| self.get(id))
    }

    /// Identities in backend order.
    pub fn ids(&self) -> impl ExactSizeIterator<Item = Uuid> + '_ {
        self.order.iter().copied()
    }

    /// Persistent source order, available for reconciling retained versions without copying IDs.
    #[must_use]
    pub fn order(&self) -> &Vector<Uuid> {
        &self.order
    }

    /// Number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Looks up a record irrespective of its position.
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&T> {
        self.rows.get(id)
    }

    /// Retains the immutable record.
    #[must_use]
    pub fn get_arc(&self, id: Uuid) -> Option<Arc<T>> {
        self.rows.get_arc(id)
    }

    /// Whether both versions have the same identity order.
    #[must_use]
    pub fn same_order(&self, other: &Self) -> bool {
        self.order.ptr_eq(&other.order) || self.order == other.order
    }

    /// Returns changed identities even when publications were coalesced.
    pub fn diff_keys<'a>(&'a self, previous: &'a Self) -> impl Iterator<Item = Uuid> + 'a {
        self.rows.diff_keys(&previous.rows)
    }

    pub(crate) fn replace(&mut self, records: Vec<T>, key: impl Fn(&T) -> Uuid) {
        let order: Vector<_> = records.iter().map(&key).collect();
        if self.order != order {
            let retained: std::collections::HashSet<_> = order.iter().copied().collect();
            for old in &self.order {
                if !retained.contains(old) {
                    self.rows.remove(*old);
                }
            }
            self.order = order;
        }
        for record in records {
            self.rows.set(key(&record), record);
        }
    }

    pub(crate) fn set(&mut self, id: Uuid, row: T) {
        if self.get(id).is_none() {
            self.order.push_back(id);
        }
        self.rows.set(id, row);
    }

    pub(crate) fn remove(&mut self, id: Uuid) {
        if self.get(id).is_some() {
            self.rows.remove(id);
            remove_from_order(&mut self.order, id);
        }
    }
}

#[cfg(test)]
impl<T: Clone + Eq> EntityTable<T> {
    /// Equal slot addresses in different retained maps prove the enclosing B-tree storage is shared.
    pub(super) fn shared_value_slots(&self, other: &Self) -> usize {
        self.0
            .iter()
            .filter(|(id, slot)| {
                other
                    .0
                    .get(*id)
                    .is_some_and(|other| std::ptr::eq(*slot, other))
            })
            .count()
    }
}

#[cfg(test)]
impl ResourceIds {
    pub(super) fn shares_order_root(&self, other: &Self) -> bool {
        self.order.ptr_eq(&other.order)
    }
}

#[cfg(test)]
impl<T: Clone + Eq> ResourceTable<T> {
    pub(super) fn shares_entity_root(&self, other: &Self) -> bool {
        self.rows.0.ptr_eq(&other.rows.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_removal_shares_the_retained_order_vector_leaves() {
        let mut base = ResourceIds::default();
        base.replace((0..50_000).map(Uuid::from_u128));
        for index in [0, 25_000] {
            let mut edited = base.clone();
            edited.remove(Uuid::from_u128(index));
            assert_eq!(edited.len(), base.len() - 1);
            assert!(!edited.contains(Uuid::from_u128(index)));
            assert!(std::ptr::eq(
                std::ptr::from_ref(&edited.order[48_000]),
                std::ptr::from_ref(&base.order[48_001])
            ));
        }
    }
}
