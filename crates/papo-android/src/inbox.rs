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
            read: n.read,
            at: n
                .created_at
                .map(|t| t.timestamp_millis())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
    result
}
#[uniffi::export]
impl NativeClient {
    pub fn inbox(&self, owner: String) -> Result<Vec<InboxItem>, CoreError> {
        {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_ref() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                return Ok(project(&c.notifications, &c.store));
            }
        }
        let request = self.scoped_api(&owner)?;
        let items = account::run(request.api.notifications(&owner))?;
        self.finish_request(&request, vec![Update::Notifications(items.clone())])?;
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let Some(Backend::Live(c)) = backend.as_ref() else {
            return Err("Account changed".to_owned().into());
        };
        if c.runtime.store.me != owner {
            return Err("Account changed".to_owned().into());
        }
        Ok(project(&items, &c.runtime.store))
    }
    pub fn inbox_read(&self, owner: String, ids: Vec<String>) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
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
                c.store
                    .apply(Update::Notifications(c.notifications.clone()));
                return Ok(());
            }
        }
        let request = self.scoped_api(&owner)?;
        account::run(request.api.mark_notifications_read(&owner, ids.clone()))?;
        let read = ids
            .into_iter()
            .map(|id| models::Notification {
                id,
                message_id: None,
                channel_id: None,
                author_id: None,
                message_content: None,
                read: true,
                created_at: None,
            })
            .collect();
        self.finish_request(&request, vec![Update::Notifications(read)])
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
