//! Narrow diagnostic projections from the session actor to the foreground.
//!
//! The actor borrows the selected core timeline after a bounded drain. Only
//! changed ID/text rows cross the thread boundary; unchanged rows keep their
//! immutable `Arc`. This is not the feature-complete message renderer.
//! Core owns canonical state, while the session layer owns identity checks
//! and ordered, lossless delivery of these deltas. A replacement consumer
//! needs a new builder so its first update includes the whole projection.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_core::api::ws::Connection;
use papo_core::state::{Screen, Store};

/// The diagnostic message fields shared immutably with the foreground.
#[derive(PartialEq, Eq)]
pub(crate) struct MessageRow {
    id: Arc<str>,
    text: String,
}

impl MessageRow {
    /// The stable core message identity.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The diagnostic text, without attachment or renderer state.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

/// Diagnostic account, connection and selection delivered to the foreground.
#[derive(PartialEq, Eq)]
pub(crate) struct SessionStatus {
    screen: Screen,
    connection: Connection,
    account_id: String,
    selected_channel: String,
}

impl Default for SessionStatus {
    fn default() -> Self {
        Self {
            screen: Screen::Starting,
            connection: Connection::Offline,
            account_id: String::new(),
            selected_channel: String::new(),
        }
    }
}

impl SessionStatus {
    /// The core's current authentication screen.
    pub(crate) fn screen(&self) -> Screen {
        self.screen
    }

    /// The core's current transport state.
    pub(crate) fn connection(&self) -> Connection {
        self.connection
    }

    /// The authenticated account ID, or empty before authentication.
    pub(crate) fn account_id(&self) -> &str {
        &self.account_id
    }

    /// The selected channel ID, or empty when none is selected.
    pub(crate) fn selected_channel(&self) -> &str {
        &self.selected_channel
    }
}

/// An ordered delta; the session layer must not discard an intermediate delta.
#[derive(Clone)]
pub(crate) struct ProjectionUpdate {
    status: Option<Arc<SessionStatus>>,
    upserts: Vec<Arc<MessageRow>>,
    removals: Vec<Arc<str>>,
    order: Option<Arc<[Arc<str>]>>,
}

#[cfg(test)]
impl ProjectionUpdate {
    /// Changed session fields, or `None` if they are unchanged.
    pub(crate) fn status(&self) -> Option<&SessionStatus> {
        self.status.as_deref()
    }

    /// New and changed rows; unchanged rows are never included.
    pub(crate) fn upserts(&self) -> &[Arc<MessageRow>] {
        &self.upserts
    }

    /// IDs no longer in the selected timeline.
    pub(crate) fn removals(&self) -> &[Arc<str>] {
        &self.removals
    }

    /// Complete ID order only when insertion, removal or reordering changed it.
    pub(crate) fn order(&self) -> Option<&[Arc<str>]> {
        self.order.as_deref()
    }
}

/// Foreground-owned diagnostic state, without a reference to the core Store.
#[derive(Default)]
pub(crate) struct Projection {
    status: Arc<SessionStatus>,
    rows: HashMap<Arc<str>, Arc<MessageRow>>,
    order: Arc<[Arc<str>]>,
}

impl Projection {
    /// Applies a delta and reports whether the foreground needs notifying.
    pub(crate) fn apply(&mut self, update: ProjectionUpdate) -> bool {
        let mut changed = false;
        if let Some(status) = update.status
            && self.status != status
        {
            self.status = status;
            changed = true;
        }
        for id in update.removals {
            changed |= self.rows.remove(&id).is_some();
        }
        for row in update.upserts {
            if self.rows.get(row.id()).is_none_or(|old| **old != *row) {
                self.rows.insert(Arc::clone(&row.id), row);
                changed = true;
            }
        }
        if let Some(order) = update.order
            && self.order != order
        {
            self.order = order;
            changed = true;
        }
        changed
    }

    /// Current account, connection and channel selection.
    pub(crate) fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// Stable IDs in the selected core timeline's order.
    pub(crate) fn order(&self) -> &[Arc<str>] {
        &self.order
    }

    /// A retained row by stable ID, without cloning its text.
    pub(crate) fn row(&self, id: &str) -> Option<&Arc<MessageRow>> {
        self.rows.get(id)
    }
}

/// Actor-owned comparison state for incremental diagnostic publication.
#[derive(Default)]
pub(crate) struct ProjectionBuilder {
    previous: Projection,
    initialized: bool,
}

impl ProjectionBuilder {
    /// Borrows the selected timeline after a bounded drain; no-op yields `None`.
    ///
    /// Core has no general content revision. Comparison scans borrowed IDs and
    /// text, but copies text only for changed rows and order only on structural
    /// changes. Pins, attachments and other fields are deliberately not drawn.
    pub(crate) fn build(&mut self, store: &Store) -> Option<ProjectionUpdate> {
        let old_status = self.previous.status();
        let status = (!self.initialized
            || old_status.screen() != store.screen
            || old_status.connection() != store.connection
            || old_status.account_id() != store.me
            || old_status.selected_channel() != store.selected_channel)
            .then(|| {
                Arc::new(SessionStatus {
                    screen: store.screen,
                    connection: store.connection,
                    account_id: store.me.clone(),
                    selected_channel: store.selected_channel.clone(),
                })
            });
        let mut ids = Vec::new();
        let mut upserts = Vec::new();
        for message in store.messages_in(&store.selected_channel) {
            ids.push(message.id.as_str());
            if self
                .previous
                .row(&message.id)
                .is_some_and(|row| row.text() == message.content)
            {
                continue;
            }
            let id = self
                .previous
                .row(&message.id)
                .map_or_else(|| Arc::from(message.id.as_str()), |row| Arc::clone(&row.id));
            let row = Arc::new(MessageRow {
                id,
                text: message.content.clone(),
            });
            self.previous
                .rows
                .insert(Arc::clone(&row.id), Arc::clone(&row));
            upserts.push(row);
        }
        let seen: HashSet<_> = ids.iter().copied().collect();
        let mut removals = Vec::new();
        self.previous.rows.retain(|id, _| {
            if seen.contains(id.as_ref()) {
                true
            } else {
                removals.push(Arc::clone(id));
                false
            }
        });
        let order = (!self
            .previous
            .order()
            .iter()
            .map(AsRef::as_ref)
            .eq(ids.iter().copied()))
        .then(|| {
            ids.iter()
                .filter_map(|id| self.previous.rows.get(*id).map(|row| Arc::clone(&row.id)))
                .collect::<Arc<[Arc<str>]>>()
        });
        if let Some(status) = &status {
            self.previous.status = Arc::clone(status);
        }
        if let Some(order) = &order {
            self.previous.order = Arc::clone(order);
        }
        self.initialized = true;
        if status.is_none() && upserts.is_empty() && removals.is_empty() && order.is_none() {
            return None;
        }
        Some(ProjectionUpdate {
            status,
            upserts,
            removals,
            order,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let mut store = Store::default();
        store.selected_channel = "channel-1".into();
        store.messages = ["message-1", "message-2"]
            .into_iter()
            .map(|id| papo_core::state::Message {
                id: id.into(),
                channel_id: "channel-1".into(),
                author_id: "account-1".into(),
                external_author: None,
                content: String::new(),
                at: "2000-01-01T00:00:00Z".parse().unwrap(),
                edited: false,
                reply_to: None,
                attachments: Vec::new(),
                previews: Vec::new(),
                reactions: Vec::new(),
                pinned: false,
                pending: false,
            })
            .collect();
        store
    }

    #[test]
    fn one_edit_replaces_only_its_row_and_keeps_order() {
        let mut store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        projection.apply(builder.build(&store).unwrap());
        let first = Arc::clone(projection.row("message-1").unwrap());
        let second = Arc::clone(projection.row("message-2").unwrap());

        store.edit_message_local("message-1", "synthetic-edit".into());
        let update = builder.build(&store).unwrap();
        assert_eq!(update.upserts().len(), 1);
        assert!(update.order().is_none());
        assert!(projection.apply(update));
        assert!(!Arc::ptr_eq(&first, projection.row("message-1").unwrap()));
        assert!(Arc::ptr_eq(&second, projection.row("message-2").unwrap()));
        assert_eq!(
            projection.row("message-1").unwrap().text(),
            "synthetic-edit"
        );
    }

    #[test]
    fn no_op_keeps_row_arcs_and_emits_nothing() {
        let store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        projection.apply(builder.build(&store).unwrap());
        let first = Arc::clone(projection.row("message-1").unwrap());

        assert!(builder.build(&store).is_none());
        assert!(Arc::ptr_eq(&first, projection.row("message-1").unwrap()));
    }

    #[test]
    fn deletion_emits_only_removed_id_and_structural_order() {
        let mut store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        projection.apply(builder.build(&store).unwrap());

        store.delete_message_local("message-1");
        let update = builder.build(&store).unwrap();
        assert!(update.upserts().is_empty());
        assert_eq!(update.removals()[0].as_ref(), "message-1");
        assert_eq!(update.order().unwrap().len(), 1);
        projection.apply(update);
        assert!(projection.row("message-1").is_none());
    }

    #[test]
    fn duplicate_delivery_does_not_notify_or_replace_rows() {
        let store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        let update = builder.build(&store).unwrap();
        assert!(projection.apply(update.clone()));
        let row = Arc::clone(projection.row("message-1").unwrap());

        assert!(!projection.apply(update));
        assert!(Arc::ptr_eq(&row, projection.row("message-1").unwrap()));
    }

    #[test]
    fn channel_switch_removes_old_rows_without_copying_other_channels() {
        let mut store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        projection.apply(builder.build(&store).unwrap());

        store.selected_channel = "channel-2".into();
        projection.apply(builder.build(&store).unwrap());
        assert!(projection.order().is_empty());
        assert!(projection.row("message-1").is_none());
        assert_eq!(projection.status().selected_channel(), "channel-2");
    }

    #[test]
    fn account_and_connection_change_without_replacing_rows() {
        let mut store = store();
        let mut builder = ProjectionBuilder::default();
        let mut projection = Projection::default();
        projection.apply(builder.build(&store).unwrap());
        let row = Arc::clone(projection.row("message-1").unwrap());

        store.me = "account-1".into();
        store.connection = Connection::Online;
        store.screen = Screen::Chat;
        let update = builder.build(&store).unwrap();
        assert!(update.status().is_some());
        assert!(update.upserts().is_empty());
        assert!(update.order().is_none());
        projection.apply(update);
        assert_eq!(projection.status().account_id(), "account-1");
        assert_eq!(projection.status().connection(), Connection::Online);
        assert_eq!(projection.status().screen(), Screen::Chat);
        assert!(Arc::ptr_eq(&row, projection.row("message-1").unwrap()));
    }
}
