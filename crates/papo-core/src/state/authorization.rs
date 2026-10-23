//! Message action policy shared by presentation and the Android mutation boundary.
use super::{ChannelKind, Message, Screen, Store};

impl Store {
    pub fn can_manage_webhooks(&self) -> bool {
        !self.me.is_empty()
            && (self.is_server_owner() || self.my_role_permissions().manage_webhooks)
    }

    pub fn can_edit_message(&self, message: &Message) -> bool {
        self.screen == Screen::Chat
            && !self.me.is_empty()
            && !message.pending
            && message.mine(&self.me)
    }

    pub fn can_delete_message(&self, message: &Message) -> bool {
        if self.screen != Screen::Chat || self.me.is_empty() || message.pending {
            return false;
        }
        if message.mine(&self.me) {
            return true;
        }
        let Some(channel) = self.channel(&message.channel_id) else {
            return false;
        };
        // Backend userHasChannelPermission(..., freeIfOpen=false): an open
        // channel grants no moderation, and server ownership never overrides DMs.
        if matches!(channel.kind, ChannelKind::Direct | ChannelKind::Category) {
            return false;
        }
        if self.is_server_owner() {
            return true;
        }
        self.member(&self.me).is_some_and(|member| {
            channel.permissions.iter().any(|entry| {
                entry.permissions.delete_messages && member.roles.contains(&entry.role_id)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{models, net::Update};
    use serde_json::json;

    fn store() -> Store {
        let mut store = Store {
            me: "reader".into(),
            screen: Screen::Chat,
            ..Default::default()
        };
        store.apply(Update::Server(Some(Box::new(
            serde_json::from_value(
                json!({"id":"server","name":"Papo","public":true,"owner_id":"owner"}),
            )
            .unwrap(),
        ))));
        store.apply(Update::Users(
            serde_json::from_value(json!([
                {"id":"reader","username":"reader","roles":[{"id":"mod","name":"Moderators"}]}
            ]))
            .unwrap(),
        ));
        store.apply(Update::Channels(
            serde_json::from_value(json!([
                {"id":"chat","name":"chat","type":"text","permissions":[]},
                {"id":"dm","name":"private","type":"dm","permissions":[]}
            ]))
            .unwrap(),
        ));
        store.apply(Update::Roles(serde_json::from_value(json!([
            {"id":"mod","name":"Moderators","permissions":{"manage_server":true,"manage_webhooks":true}},
            {"id":"unassigned","name":"Other","permissions":{"manage_roles":true}}
        ])).unwrap()));
        store
    }

    fn message(author: &str, channel: &str) -> Message {
        Message {
            id: "message".into(),
            channel_id: channel.into(),
            author_id: author.into(),
            external_author: None,
            content: "hello".into(),
            at: chrono::Local::now(),
            edited: false,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        }
    }

    #[test]
    fn moderation_requires_an_explicit_channel_grant_on_an_assigned_role() {
        let mut store = store();
        let other = message("other", "chat");
        // Neither an open channel nor global manage_server grants deletion.
        assert!(!store.can_delete_message(&other));
        let grant = models::ChannelPermissionEntry {
            role_id: "unassigned".into(),
            role_name: "Other".into(),
            permissions: models::ChannelPermissions {
                delete_messages: true,
                ..Default::default()
            },
        };
        store.channels[0].permissions = vec![grant.clone()];
        assert!(!store.can_delete_message(&other));
        store.channels[0]
            .permissions
            .push(models::ChannelPermissionEntry {
                role_id: "mod".into(),
                ..grant
            });
        assert!(store.can_delete_message(&other));
        assert!(!store.can_edit_message(&other));
        store.apply(Update::ChannelPermissions {
            channel_id: "chat".into(),
            permissions: Vec::new(),
        });
        assert!(!store.can_delete_message(&other));
    }

    #[test]
    fn authors_can_edit_and_delete_but_pending_and_signed_out_messages_cannot() {
        let mut store = store();
        let mut own = message("reader", "chat");
        assert!(store.can_edit_message(&own));
        assert!(store.can_delete_message(&own));
        own.pending = true;
        assert!(!store.can_edit_message(&own));
        assert!(!store.can_delete_message(&own));
        own.pending = false;
        store.me.clear();
        own.author_id.clear();
        assert!(!store.can_edit_message(&own));
        assert!(!store.can_delete_message(&own));
    }

    #[test]
    fn owner_override_is_limited_to_server_channels_and_never_edits_other_authors() {
        let mut store = store();
        store.me = "owner".into();
        assert!(store.can_delete_message(&message("other", "chat")));
        assert!(!store.can_edit_message(&message("other", "chat")));
        assert!(!store.can_delete_message(&message("other", "dm")));
        assert!(store.can_delete_message(&message("owner", "dm")));
        assert!(!store.can_delete_message(&message("other", "missing")));
    }

    #[test]
    fn webhook_permission_aggregates_only_assigned_roles_and_owner_privileges() {
        let mut store = store();
        assert!(store.my_role_permissions().manage_webhooks);
        assert!(!store.my_role_permissions().manage_roles);
        assert!(store.can_manage_webhooks());
        store.members[0].roles.clear();
        assert!(!store.can_manage_webhooks());
        store.me = "owner".into();
        assert!(store.can_manage_webhooks());
    }
}
