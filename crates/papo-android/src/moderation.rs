//! A deletion is projected only after the backend accepts it.
use crate::{
    account,
    bindings::{Backend, CoreError, NativeClient},
};
use papo_core::api::net::Update;

#[uniffi::export]
impl NativeClient {
    pub fn delete_message(&self, owner: String, message_id: String) -> Result<(), CoreError> {
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let backend = backend.as_mut().ok_or("Client is closed".to_owned())?;
            let store = match backend {
                Backend::Live(client) => &mut client.runtime.store,
                Backend::Demo(client) => &mut client.store,
            };
            if store.me != owner
                || !store
                    .preview_message(&message_id)
                    .is_some_and(|m| store.can_delete_message(m))
            {
                return Err("You cannot delete this message".to_owned().into());
            }
            if let Backend::Demo(client) = backend {
                client.store.apply(Update::Deleted(message_id));
                return Ok(());
            }
        }
        let request = self.scoped_api(&owner)?;
        account::run(request.api.delete_message(&message_id))?;
        // The frozen request identity prevents a late response from changing the
        // next account. Deleted projects through history, detached results and pins.
        self.finish_request(&request, vec![Update::Deleted(message_id)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::administration::AdministrationChange;

    #[test]
    fn acknowledged_role_snapshot_preserves_webhook_permission_through_a_rename() {
        let client = NativeClient::demo();
        let role = client.poll().unwrap().roles.into_iter().next().unwrap();
        let mut permissions = role.permissions;
        permissions.manage_webhooks = true;
        client
            .administer(
                "me".into(),
                AdministrationChange::UpdateRole {
                    id: role.id.clone(),
                    name: role.name.clone(),
                    color: role.color.clone(),
                    permissions,
                },
            )
            .unwrap();
        let loaded = client
            .administration("me".into(), false)
            .unwrap()
            .roles
            .into_iter()
            .find(|r| r.id == role.id)
            .unwrap();
        assert!(loaded.permissions.manage_webhooks);
        client
            .administer(
                "me".into(),
                AdministrationChange::UpdateRole {
                    id: loaded.id.clone(),
                    name: "Renamed".into(),
                    color: loaded.color,
                    permissions: loaded.permissions,
                },
            )
            .unwrap();
        let result = client
            .poll()
            .unwrap()
            .roles
            .into_iter()
            .find(|r| r.id == role.id)
            .unwrap();
        assert_eq!(result.name, "Renamed");
        assert!(result.permissions.manage_webhooks);
    }
}
