//! Owns counted live gateway presence and bounded updates; leaves identity and socket ownership outside.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::sync::broadcast;

#[derive(Clone)]
pub(crate) struct Update {
    pub(crate) recipient: Option<String>,
    pub(crate) payload: Value,
}

pub(crate) struct Presence {
    connections: BTreeMap<String, usize>,
    updates: broadcast::Sender<Update>,
}

impl Presence {
    pub(crate) fn new() -> Self {
        let (updates, _) = broadcast::channel(256);
        Self {
            connections: BTreeMap::new(),
            updates,
        }
    }

    pub(crate) fn join(&mut self, user_id: &str) -> (Value, broadcast::Receiver<Update>) {
        let count = self.connections.entry(user_id.to_owned()).or_default();
        *count += 1;
        if *count == 1 {
            self.emit(None, update(user_id, "online"));
        }
        let updates = self.updates.subscribe();
        let members: Vec<_> = self.connections.keys().map(|user_id| json!({"user_id": user_id, "status": "online", "status_message": null, "user_voice": []})).collect();
        (
            json!({"type": "presence_sync", "members": members}),
            updates,
        )
    }

    pub(crate) fn user_registered(&self, user_id: &str) {
        self.emit(None, json!({"type": "user_join", "user_id": user_id}));
    }

    pub(crate) fn emit(&self, recipient: Option<&str>, payload: Value) {
        let _ = self.updates.send(Update {
            recipient: recipient.map(str::to_owned),
            payload,
        });
    }

    pub(crate) fn leave(&mut self, user_id: &str) {
        let Some(count) = self.connections.get_mut(user_id) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            self.connections.remove(user_id);
            self.emit(None, update(user_id, "offline"));
        }
    }
}

fn update(user_id: &str, status: &str) -> Value {
    json!({"type": "presence_update", "user_id": user_id, "status": status, "status_message": null, "typing": null, "nickname": null})
}

#[cfg(test)]
mod tests {
    //! Owns pure counted-presence tests; leaves connection lifetime and delivery to integration tests.

    use super::*;

    #[test]
    fn second_connection_does_not_duplicate_members_or_online_updates() {
        let mut presence = Presence::new();
        let (_, mut updates) = presence.join("member");
        let (snapshot, _) = presence.join("member");
        assert_eq!(snapshot["members"].as_array().map(Vec::len), Some(1));
        assert!(matches!(
            updates.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn only_last_connection_leaving_reports_offline() {
        let mut presence = Presence::new();
        let (_, mut updates) = presence.join("member");
        presence.join("member");
        presence.leave("member");
        assert!(matches!(
            updates.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        presence.leave("member");
        assert_eq!(updates.try_recv().unwrap().payload["status"], "offline");
    }
}
