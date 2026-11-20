//! Owns counted live gateway presence and bounded updates; leaves identity and socket ownership outside.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use tokio::sync::broadcast;

#[derive(Clone)]
pub(crate) struct Update {
    recipient: Option<String>,
    recipient_client: Option<String>,
    excluded: BTreeSet<String>,
    pub(crate) payload: Value,
}

impl Update {
    #[cfg(test)]
    pub(crate) fn visible_to(&self, user_id: &str) -> bool {
        self.visible_to_client(user_id, "")
    }

    pub(crate) fn visible_to_client(&self, user_id: &str, client_id: &str) -> bool {
        if let Some(target_client) = self.recipient_client.as_deref() {
            return target_client == client_id;
        }
        self.recipient
            .as_deref()
            .is_none_or(|recipient| recipient == user_id)
            && !self.excluded.contains(user_id)
    }
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

    #[cfg(test)]
    pub(crate) fn join(&mut self, user_id: &str) -> (Value, broadcast::Receiver<Update>) {
        self.join_with_voice(user_id, &BTreeMap::new())
    }

    pub(crate) fn join_with_voice(
        &mut self,
        user_id: &str,
        voice_by_user: &BTreeMap<String, Vec<String>>,
    ) -> (Value, broadcast::Receiver<Update>) {
        let count = self.connections.entry(user_id.to_owned()).or_default();
        *count += 1;
        if *count == 1 {
            self.emit(None, update(user_id, "online"));
        }
        let updates = self.updates.subscribe();
        let mut users: BTreeSet<&str> = self.connections.keys().map(String::as_str).collect();
        for member_id in voice_by_user.keys() {
            users.insert(member_id.as_str());
        }
        let members: Vec<_> = users
            .into_iter()
            .map(|member_id| {
                let channels = voice_by_user.get(member_id).cloned().unwrap_or_default();
                json!({
                    "user_id": member_id,
                    "status": "online",
                    "status_message": null,
                    "user_voice": channels,
                })
            })
            .collect();
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
            recipient_client: None,
            excluded: BTreeSet::new(),
            payload,
        });
    }

    pub(crate) fn emit_to_client(&self, client_id: &str, payload: Value) {
        let _ = self.updates.send(Update {
            recipient: None,
            recipient_client: Some(client_id.to_owned()),
            excluded: BTreeSet::new(),
            payload,
        });
    }

    pub(crate) fn emit_except(&self, excluded: BTreeSet<String>, payload: Value) {
        let _ = self.updates.send(Update {
            recipient: None,
            recipient_client: None,
            excluded,
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

#[cfg(test)]
mod audience_tests {
    //! Owns pure account filtering and bounded-broadcast proofs; leaves frame transport outside.
    use super::*;

    #[test]
    fn filtered_history_event_is_one_broadcast_not_one_per_reader() {
        let mut presence = Presence::new();
        let (_, mut updates) = presence.join("reader");
        let excluded = (0..10_000).map(|index| format!("denied-{index}")).collect();
        presence.emit_except(excluded, json!({"type": "heartbeat_ack"}));
        let update = updates.try_recv().unwrap();
        assert!(update.visible_to("reader"));
        assert!(!update.visible_to("denied-9999"));
        assert!(matches!(
            updates.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}
