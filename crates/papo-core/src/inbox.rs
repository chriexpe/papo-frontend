//! Account-local inbox pagination. Read flags are monotonic; live head refreshes
//! cannot discard older loaded pages or let stale pages undo read acknowledgements.
use crate::api::models::{Notification, NotificationList};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    pub at: DateTime<Utc>,
    pub id: String,
}
impl Cursor {
    pub fn from_notification(n: &Notification) -> Option<Self> {
        Some(Self {
            at: n.created_at?,
            id: n.id.clone(),
        })
    }
}
#[derive(Clone, Debug)]
pub struct Flight {
    generation: u64,
    pub cursor: Option<Cursor>,
    more: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum PageError {
    Busy,
    Exhausted,
    NoProgress,
    Stale,
}
#[derive(Default, Debug)]
pub struct Inbox {
    items: HashMap<String, Notification>,
    deleted_messages: std::collections::HashSet<String>,
    pub cursor: Option<Cursor>,
    pub has_more: bool,
    loaded: bool,
    generation: u64,
    flight: Option<u64>,
}
impl Inbox {
    pub fn current(&self, flight: &Flight) -> bool { self.flight == Some(flight.generation) }
    pub fn loading(&self) -> bool { self.flight.is_some() }
    pub fn loaded(&self) -> bool { self.loaded }

    /// Live records do not advance the HTTP cursor or undo acknowledged reads/deletions.
    pub fn observe(&mut self, items: &[Notification]) {
        if !self.loaded { return; }
        for item in items {
            if item.message_id.as_deref().is_some_and(|id| self.message_deleted(id)) { continue; }
            let mut item = item.clone();
            item.read |= self.items.get(&item.id).is_some_and(|old| old.read);
            self.items.insert(item.id.clone(), item);
        }
    }

    pub fn reset(&mut self) {
        let generation = self.generation + 1;
        *self = Self {
            generation,
            ..Default::default()
        };
    }
    /// Credential renewal invalidates in-flight tickets, not this owner's history.
    pub fn invalidate_requests(&mut self) {
        self.generation += 1;
        self.flight = None;
    }
    pub fn begin(&mut self, more: bool) -> Result<Flight, PageError> {
        if self.flight.is_some() {
            return Err(PageError::Busy);
        }
        if more && (!self.has_more || self.cursor.is_none()) {
            return Err(PageError::Exhausted);
        }
        self.generation += 1;
        self.flight = Some(self.generation);
        Ok(Flight {
            generation: self.generation,
            cursor: if more { self.cursor.clone() } else { None },
            more,
        })
    }
    pub fn cancel(&mut self, request: &Flight) {
        if self.flight == Some(request.generation) {
            self.flight = None;
        }
    }
    pub fn complete(&mut self, request: &Flight, page: NotificationList) -> Result<(), PageError> {
        if self.flight != Some(request.generation) {
            return Err(PageError::Stale);
        }
        self.flight = None;
        // Derive the cursor from the raw page, before any presentation filtering.
        let next = page
            .notifications
            .last()
            .and_then(Cursor::from_notification);
        if page.has_more && (next.is_none() || (request.more && next >= request.cursor)) {
            return Err(PageError::NoProgress);
        }
        let overlaps = page
            .notifications
            .iter()
            .any(|n| self.items.contains_key(&n.id));
        let complete_snapshot = !request.more && !page.has_more;
        if !self.loaded || request.more || complete_snapshot || !overlaps {
            self.cursor = next;
            self.has_more = page.has_more;
        }
        let ids: std::collections::HashSet<_> =
            page.notifications.iter().map(|n| n.id.clone()).collect();
        for mut n in page.notifications {
            if n.message_id
                .as_deref()
                .is_some_and(|id| self.message_deleted(id))
            {
                continue;
            }
            if self.items.get(&n.id).is_some_and(|old| old.read) {
                n.read = true;
            }
            self.items.insert(n.id.clone(), n);
        }
        if complete_snapshot {
            self.items.retain(|id, _| ids.contains(id));
        }
        self.loaded = true;
        Ok(())
    }
    pub fn remove_message(&mut self, id: &str) {
        self.deleted_messages.insert(id.into());
        self.items
            .retain(|_, n| n.message_id.as_deref() != Some(id));
    }
    pub fn message_deleted(&self, id: &str) -> bool {
        self.deleted_messages.contains(id)
    }
    pub fn mark_read(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(n) = self.items.get_mut(id) {
                n.read = true;
            }
        }
    }
    pub fn reconcile(&mut self, read: impl Fn(&Notification) -> bool) {
        for n in self.items.values_mut() {
            n.read |= read(n);
        }
    }
    pub fn items(&self) -> Vec<Notification> {
        let mut items = self.items.values().cloned().collect::<Vec<_>>();
        items.sort_by_key(|n| std::cmp::Reverse(Cursor::from_notification(n)));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(id: &str, nanos: u32, read: bool) -> Notification {
        serde_json::from_value(serde_json::json!({"id":id,"created_at":format!("2026-10-23T12:00:00.{nanos:09}Z"),"read":read})).unwrap()
    }
    fn page(items: Vec<Notification>, has_more: bool) -> NotificationList {
        NotificationList {
            notifications: items,
            has_more,
        }
    }
    #[test]
    fn refresh_keeps_old_cursor_read_acks_and_exact_submillisecond_order() {
        let mut inbox = Inbox::default();
        let first = inbox.begin(false).unwrap();
        assert_eq!(inbox.begin(false).unwrap_err(), PageError::Busy);
        inbox
            .complete(&first, page(vec![n("b", 9, false), n("a", 9, false)], true))
            .unwrap();
        let more = inbox.begin(true).unwrap();
        inbox.mark_read(&["a".into()]);
        inbox
            .complete(&more, page(vec![n("a", 9, false), n("old", 1, true)], true))
            .unwrap();
        let cursor = inbox.cursor.clone();
        let head = inbox.begin(false).unwrap();
        inbox
            .complete(
                &head,
                page(vec![n("new", 10, false), n("a", 9, false)], true),
            )
            .unwrap();
        assert_eq!(inbox.cursor, cursor);
        assert_eq!(
            inbox
                .items()
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "b", "a", "old"]
        );
        assert!(inbox.items().iter().find(|n| n.id == "a").unwrap().read);
        let more = inbox.begin(true).unwrap();
        assert_eq!(
            inbox.complete(&more, page(vec![n("old", 1, false)], true)),
            Err(PageError::NoProgress)
        );
        assert!(inbox.has_more);
        let retry = inbox.begin(true).unwrap();
        inbox.complete(&retry, page(vec![], false)).unwrap();
        assert!(!inbox.has_more);
    }
    #[test]
    fn deletion_during_a_page_request_keeps_its_raw_cursor_without_resurrecting_the_row() {
        let mut inbox = Inbox::default();
        let head = inbox.begin(false).unwrap();
        let mut item = n("notice", 1, false);
        item.message_id = Some("deleted".into());
        inbox.remove_message("deleted");
        inbox.complete(&head, page(vec![item], true)).unwrap();
        assert!(inbox.items().is_empty());
        assert!(inbox.has_more);
        assert_eq!(inbox.cursor.as_ref().unwrap().id, "notice");
    }
    #[test]
    fn credential_renewal_retains_loaded_history_but_rejects_the_old_page() {
        let mut inbox = Inbox::default();
        let head = inbox.begin(false).unwrap();
        inbox
            .complete(&head, page(vec![n("cached", 5, true)], true))
            .unwrap();
        let old = inbox.begin(true).unwrap();
        inbox.invalidate_requests();
        let retry = inbox.begin(true).unwrap();
        assert_eq!(
            inbox.complete(&old, page(vec![n("stale", 1, false)], false)),
            Err(PageError::Stale)
        );
        inbox
            .complete(&retry, page(vec![n("older", 1, false)], false))
            .unwrap();
        assert_eq!(inbox.items().len(), 2);
        assert!(inbox.items().iter().any(|n| n.id == "cached" && n.read));
    }
    #[test]
    fn reset_never_reuses_an_old_requests_ticket() {
        let mut inbox = Inbox::default();
        let old = inbox.begin(false).unwrap();
        inbox.reset();
        let new = inbox.begin(false).unwrap();
        inbox.cancel(&old);
        assert_eq!(inbox.begin(false).unwrap_err(), PageError::Busy);
        assert_eq!(
            inbox.complete(&old, page(vec![], false)),
            Err(PageError::Stale)
        );
        inbox
            .complete(&new, page(vec![n("new-owner", 1, false)], false))
            .unwrap();
        assert_eq!(inbox.items()[0].id, "new-owner");
    }
    #[test]
    fn cancelled_results_are_stale_and_a_disjoint_head_does_not_skip_intermediate_pages() {
        let mut inbox = Inbox::default();
        let old = inbox.begin(false).unwrap();
        inbox.cancel(&old);
        let head = inbox.begin(false).unwrap();
        assert_eq!(
            inbox.complete(&old, page(vec![n("stale", 1, false)], false)),
            Err(PageError::Stale)
        );
        inbox
            .complete(&head, page(vec![n("old", 1, false)], true))
            .unwrap();
        let head = inbox.begin(false).unwrap();
        inbox
            .complete(&head, page(vec![n("new", 10, false)], true))
            .unwrap();
        assert_eq!(inbox.cursor.as_ref().unwrap().id, "new");
        assert_eq!(inbox.items().len(), 2);
        let more = inbox.begin(true).unwrap();
        inbox
            .complete(
                &more,
                page(vec![n("between", 5, false), n("old", 1, false)], false),
            )
            .unwrap();
        assert_eq!(inbox.items().len(), 3);
    }
}
