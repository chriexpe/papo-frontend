//! The global inbox reads backend records through the same authenticated core
//! API scope as account operations. Android never manufactures notifications.
use super::*;
use bindings::{Backend, CoreError, NativeClient};
use papo_core::api::models;

#[derive(Clone, Debug, uniffi::Record)]
pub struct InboxItem {
    pub id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub message_id: String,
    pub author_id: String,
    pub author: String,
    pub content: String,
    pub read: bool,
    pub at: i64,
    pub order_time: String,
}
fn project(items: &[models::Notification], store: &papo_core::state::Store) -> Vec<InboxItem> {
    let mut result = items
        .iter()
        .filter(|n| n.author_id.as_deref() != Some(&store.me))
        .map(|n| InboxItem {
            id: n.id.clone(),
            channel_id: n.channel_id.clone().unwrap_or_default(),
            message_id: n.message_id.clone().unwrap_or_default(),
            channel_name: n
                .channel_id
                .as_deref()
                .and_then(|id| store.channel(id))
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            author_id: n.author_id.clone().unwrap_or_default(),
            author: n
                .author_id
                .as_deref()
                .and_then(|id| store.member(id))
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            content: store.display_mentions(n.message_content.as_deref().unwrap_or_default()),
            read: n.read || store.notification_was_read(n),
            order_time: n
                .created_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
                .unwrap_or_default(),
            at: n
                .created_at
                .map(|t| t.timestamp_millis())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| {
        b.order_time
            .cmp(&a.order_time)
            .then_with(|| b.id.cmp(&a.id))
    });
    result
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct InboxCursor {
    pub since: String,
    pub last_id: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct InboxPage {
    pub items: Vec<InboxItem>,
    pub cursor: Option<InboxCursor>,
    pub has_more: bool,
}

#[derive(Default)]
pub(super) struct Pager {
    owner: String,
    token: Option<String>,
    state: papo_core::inbox::Inbox,
}
impl Pager {
    fn scope(&mut self, owner: &str, token: Option<String>) {
        if self.owner != owner {
            self.state.reset();
        } else if self.token != token {
            self.state.invalidate_requests();
        }
        self.owner = owner.into();
        self.token = token;
    }
    fn snapshot(&mut self, store: &papo_core::state::Store) -> InboxPage {
        self.state.reconcile(|n| store.notification_was_read(n));
        InboxPage {
            items: project(&self.state.items(), store),
            cursor: self.state.cursor.as_ref().map(|c| InboxCursor {
                since: c.at.to_rfc3339(),
                last_id: c.id.clone(),
            }),
            has_more: self.state.has_more,
        }
    }
}
impl crate::Client {
    pub(super) fn observe_inbox_update(&mut self, update: &Update) {
        use papo_core::api::ws::Event;
        match update {
            Update::Notifications(_) => self.inbox_revision += 1,
            Update::Event(event) if matches!(event.as_ref(), Event::Notification { .. }) => {
                self.inbox_revision += 1
            }
            Update::Deleted(id) => {
                self.inbox.state.remove_message(id);
                self.inbox_revision += 1;
            }
            Update::Event(event) => {
                if let Event::MessageDeleted { id, .. } = event.as_ref() {
                    self.inbox.state.remove_message(id);
                    self.inbox_revision += 1;
                }
            }
            _ => {}
        }
    }
}
fn page_error(error: papo_core::inbox::PageError) -> CoreError {
    format!("Inbox page: {error:?}").into()
}

#[uniffi::export]
impl NativeClient {
    pub fn inbox_snapshot(&self, owner: String) -> Result<InboxPage, CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        match backend.as_mut().ok_or("Client is closed".to_owned())? {
            Backend::Live(c) => {
                c.inbox
                    .scope(&c.runtime.store.me, c.runtime.net.session.token());
                if c.runtime.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                Ok(c.inbox.snapshot(&c.runtime.store))
            }
            Backend::Demo(c) => {
                c.inbox.scope(&c.store.me, None);
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                Ok(c.inbox.snapshot(&c.store))
            }
        }
    }
    pub fn inbox(&self, owner: String) -> Result<Vec<InboxItem>, CoreError> {
        Ok(self.inbox_page(owner, false)?.items)
    }
    pub fn inbox_page(&self, owner: String, more: bool) -> Result<InboxPage, CoreError> {
        let (flight, started_token) = {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            match backend.as_mut().ok_or("Client is closed".to_owned())? {
                Backend::Demo(c) => {
                    if c.store.me != owner {
                        return Err("Account changed".to_owned().into());
                    }
                    c.inbox.scope(&owner, None);
                    let flight = c.inbox.state.begin(more).map_err(page_error)?;
                    let mut items = c.notifications.clone();
                    items.sort_by_key(|n| {
                        std::cmp::Reverse(papo_core::inbox::Cursor::from_notification(n))
                    });
                    if let Some(cursor) = &flight.cursor {
                        items.retain(|n| {
                            papo_core::inbox::Cursor::from_notification(n)
                                .as_ref()
                                .is_some_and(|c| c < cursor)
                        });
                    }
                    let has_more = items.len() > 100;
                    items.truncate(100);
                    c.store.apply(Update::Notifications(items.clone()));
                    c.inbox
                        .state
                        .complete(
                            &flight,
                            models::NotificationList {
                                notifications: items,
                                has_more,
                            },
                        )
                        .map_err(page_error)?;
                    return Ok(c.inbox.snapshot(&c.store));
                }
                Backend::Live(c) => {
                    c.inbox
                        .scope(&c.runtime.store.me, c.runtime.net.session.token());
                    if c.runtime.store.me != owner {
                        return Err("Account changed".to_owned().into());
                    }
                    (
                        c.inbox.state.begin(more).map_err(page_error)?,
                        c.runtime.net.session.token(),
                    )
                }
            }
        };
        let result = (|| {
            let request = self.scoped_api(&owner)?;
            if !request.uses_token(&started_token) {
                return Err("Account changed".to_owned().into());
            }
            let page = account::run(
                request
                    .api
                    .notification_page(&owner, flight.cursor.as_ref()),
            )?;
            self.finish_request_with(&request, vec![], |c| {
                let mut page = page;
                for n in &mut page.notifications {
                    n.read |= c.runtime.store.notification_was_read(n);
                }
                let updates = page
                    .notifications
                    .iter()
                    .filter(|n| {
                        !n.message_id
                            .as_deref()
                            .is_some_and(|id| c.inbox.state.message_deleted(id))
                    })
                    .cloned()
                    .collect();
                c.inbox.state.complete(&flight, page).map_err(page_error)?;
                c.runtime.store.apply(Update::Notifications(updates));
                c.inbox.token = c.runtime.net.session.token();
                Ok(c.inbox.snapshot(&c.runtime.store))
            })
        })();
        if result.is_err()
            && let Ok(mut backend) = self.backend.lock()
            && let Some(Backend::Live(c)) = backend.as_mut()
        {
            c.inbox.state.cancel(&flight);
        }
        result
    }
    /// One acknowledged backend batch. The caller splits loaded IDs at 1000;
    /// each successful batch is retained even if a later batch fails.
    pub fn inbox_read(&self, owner: String, ids: Vec<String>) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        if ids.len() > 1000 || ids.iter().any(String::is_empty) {
            return Err("Invalid notification batch".to_owned().into());
        }
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_mut() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                for item in &mut c.notifications {
                    if ids.contains(&item.id) {
                        item.read = true;
                    }
                }
                c.inbox.state.mark_read(&ids);
                c.store
                    .apply(Update::Notifications(c.notifications.clone()));
                return Ok(());
            }
        }
        let request = self.scoped_api(&owner)?;
        match account::run_api(request.api.mark_notifications_read(&owner, ids.clone())) {
            Ok(_) | Err(papo_core::api::client::ApiError::NotFound) => {}
            Err(error) => return Err(error.to_string().into()),
        }
        let read = ids
            .iter()
            .map(|id| models::Notification {
                id: id.clone(),
                message_id: None,
                channel_id: None,
                author_id: None,
                message_content: None,
                read: true,
                created_at: None,
            })
            .collect();
        self.finish_request_with(&request, vec![Update::Notifications(read)], |c| {
            c.inbox.state.mark_read(&ids);
            Ok(())
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_inbox_has_real_targets_acknowledges_read_and_rejects_other_owners() {
        let client = NativeClient::demo();
        let items = client.inbox("me".into()).unwrap();
        assert!(!items.is_empty());
        let item = &items[0];
        assert!(
            client
                .poll()
                .unwrap()
                .messages
                .iter()
                .any(|m| m.id == item.message_id)
        );
        assert!(!item.read);
        assert!(client.inbox("other".into()).is_err());
        assert!(
            client
                .inbox_read("other".into(), vec![item.id.clone()])
                .is_err()
        );
        client
            .inbox_read("me".into(), vec![item.id.clone()])
            .unwrap();
        assert!(client.inbox("me".into()).unwrap()[0].read);
        assert_eq!(client.poll().unwrap().notification_count, 0);
    }
}
