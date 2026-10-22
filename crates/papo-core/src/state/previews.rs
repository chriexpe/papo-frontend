//! Complete message snapshots for result lists. These are not continuous
//! channel history: browsing them must never move a cursor or acknowledge read.
use super::*;
impl Store {
    pub fn media_messages(&self) -> impl Iterator<Item = &Message> {
        self.messages
            .iter()
            .chain(self.message_contexts.values().flat_map(|c| &c.messages))
            .chain(self.pinned_messages.values().flatten())
            .chain(&self.preview_messages)
    }

    pub fn preview_message(&self, id: &str) -> Option<&Message> {
        self.message(id)
            .or_else(|| self.pinned_messages.values().flatten().find(|m| m.id == id))
            .or_else(|| self.preview_messages.iter().find(|m| m.id == id))
    }
    /// Prefer an acknowledged detached snapshot; cached history alone is not
    /// proof that its attachments/embeds still match a fresh search response.
    pub fn search_message(&self, id: &str) -> Option<&Message> {
        let hit = self.search_results.iter().find(|r| r.id == id)?;
        self.preview_messages
            .iter()
            .find(|m| m.id == id && m.channel_id == hit.channel_id)
            .or_else(|| {
                self.message(id).filter(|m| {
                    m.channel_id == hit.channel_id
                        && m.content == hit.content
                        && self.timeline_status(&hit.channel_id) == TimelineStatus::Fresh
                })
            })
    }
    pub fn begin_message_preview(&mut self, channel: &str, id: &str) -> Option<RefreshTicket> {
        if self.preview_requests.contains_key(id)
            || !self
                .search_results
                .iter()
                .any(|r| r.id == id && r.channel_id == channel)
        {
            return None;
        }
        self.next_refresh_request_id = self.next_refresh_request_id.saturating_add(1);
        let ticket = RefreshTicket {
            channel_id: channel.into(),
            generation: self.sync_generation,
            request_id: self.next_refresh_request_id,
        };
        self.preview_requests.insert(
            id.into(),
            ContextRequest {
                ticket: ticket.clone(),
                mutations: Vec::new(),
                report_error: false,
            },
        );
        Some(ticket)
    }
    pub fn finish_message_preview(
        &mut self,
        ticket: &RefreshTicket,
        id: &str,
        message: Option<models::Message>,
    ) -> bool {
        if ticket.generation != self.sync_generation
            || self
                .preview_requests
                .get(id)
                .is_none_or(|r| &r.ticket != ticket)
        {
            return false;
        }
        let request = self
            .preview_requests
            .remove(id)
            .expect("matched preview request");
        let Some(message) = message else {
            return true;
        };
        if message.id != id || message.channel_id != ticket.channel_id {
            return false;
        }
        let mut messages = vec![convert(message, &self.me)];
        for mutation in request.mutations {
            project_messages(&mut messages, &mutation, false);
        }
        self.preview_messages.retain(|m| m.id != id);
        for mut message in messages {
            if let Some(pins) = self.known_pins.get(&message.channel_id) {
                message.pinned = pins.contains(id);
            }
            self.reconcile_known_message(&message);
            self.preview_messages.push(message);
        }
        if self.preview_messages.len() > 128 {
            self.preview_messages
                .drain(..self.preview_messages.len() - 128);
        }
        true
    }
    /// Snapshot reconciliation changes known bodies, never history coverage.
    pub(super) fn reconcile_known_message(&mut self, message: &Message) {
        let mutation = TimelineMutation::MessageUpsert(message.clone());
        project_messages(&mut self.messages, &mutation, false);
        project_messages(&mut self.preview_messages, &mutation, false);
        for context in self.message_contexts.values_mut() {
            project_messages(&mut context.messages, &mutation, false);
        }
        for pins in self.pinned_messages.values_mut() {
            project_messages(pins, &mutation, false);
        }
    }
    pub fn can_pin_in(&self, channel: &str) -> bool {
        self.channel(channel).is_some_and(|c| {
            c.kind == ChannelKind::Direct
                || self.is_server_owner()
                || self.my_role_permissions().pin_message
        })
    }
    pub fn pinned_in<'a>(&'a self, channel: &'a str) -> Vec<&'a Message> {
        let mut result: Vec<_> = self
            .messages_in(channel)
            .chain(self.pinned_messages.get(channel).into_iter().flatten())
            .chain(
                self.preview_messages
                    .iter()
                    .filter(|m| m.channel_id == channel),
            )
            .filter(|m| m.pinned)
            .collect();
        result.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
        let mut seen = HashSet::new();
        result.retain(|message| seen.insert(message.id.as_str()));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message() -> models::Message {
        serde_json::from_value(serde_json::json!({"id":"remote", "channel_id":"elsewhere", "author_id":"other", "content":"old", "created_at":"2026-10-20T12:00:00Z"})).unwrap()
    }
    fn store() -> Store {
        let mut s = Store {
            me: "me".into(),
            selected_channel: "reading".into(),
            ..Default::default()
        };
        s.search_results=vec![serde_json::from_value(serde_json::json!({"id":"remote","type":"message","channel_id":"elsewhere","content":"old","created_at":"2026-10-20T12:00:00Z"})).unwrap()];
        s
    }
    #[test]
    fn preview_is_not_history_and_pending_edits_override_http_snapshot() {
        let mut s = store();
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        s.apply(Update::Event(Box::new(Event::MessageEdited {
            id: "remote".into(),
            channel_id: "elsewhere".into(),
            content: "updated while loading".into(),
        })));
        assert!(s.finish_message_preview(&ticket, "remote", Some(message())));
        assert_eq!(
            s.preview_message("remote").unwrap().content,
            "updated while loading"
        );
        assert!(s.message("remote").is_none());
        assert!(s.messages.is_empty());
        assert_eq!(s.messages_in("elsewhere").count(), 0);
        assert_eq!(s.selected_channel, "reading");
        assert!(!s.browsing_history("elsewhere"));
        assert!(s.pending_read_receipts().is_empty());
    }
    #[test]
    fn deletion_and_new_search_cannot_resurrect_an_old_preview() {
        let mut s = store();
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        s.apply(Update::Event(Box::new(Event::MessageDeleted {
            id: "remote".into(),
            channel_id: "elsewhere".into(),
        })));
        assert!(s.finish_message_preview(&ticket, "remote", Some(message())));
        assert!(s.preview_message("remote").is_none());
        assert!(s.search_results.is_empty());
        let mut s = store();
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        s.reset_search();
        assert!(!s.finish_message_preview(&ticket, "remote", Some(message())));
        assert!(s.preview_messages.is_empty());
    }
    #[test]
    fn live_pin_and_moderation_updates_reach_detached_media_and_pin_list() {
        let mut s = store();
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        let mut message = message();
        message.attachments=serde_json::from_value(serde_json::json!([{"id":"photo","mime_type":"image/png","moderation_status":"pending"}])).unwrap();
        assert!(s.finish_message_preview(&ticket, "remote", Some(message)));
        s.apply(Update::Event(Box::new(Event::AttachmentModeration {
            message_id: "remote".into(),
            attachment_id: "photo".into(),
            status: "clean".into(),
        })));
        assert_eq!(
            s.preview_message("remote").unwrap().attachments[0]
                .moderation_status
                .as_deref(),
            Some("clean")
        );
        s.pinned_messages.insert("elsewhere".into(), Vec::new());
        s.apply(Update::Event(Box::new(Event::MessagePinned {
            message_id: "remote".into(),
            pinned: true,
        })));
        assert_eq!(s.pinned_in("elsewhere").len(), 1);
        s.apply(Update::Event(Box::new(Event::MessagePinned {
            message_id: "remote".into(),
            pinned: false,
        })));
        assert!(s.pinned_in("elsewhere").is_empty());
        assert!(!s.preview_message("remote").unwrap().pinned);
    }
    #[test]
    fn authoritative_pin_refresh_updates_preview_flags_and_disconnect_keeps_cached_media() {
        let mut s = store();
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        assert!(s.finish_message_preview(&ticket, "remote", Some(message())));
        s.apply(Update::PinChanged {
            channel_id: "elsewhere".into(),
            message_id: "remote".into(),
            pinned: true,
        });
        assert_eq!(s.pinned_in("elsewhere").len(), 1);
        s.apply(Update::Pinned {
            channel_id: "elsewhere".into(),
            ids: Vec::new(),
        });
        assert!(s.pinned_in("elsewhere").is_empty());
        assert!(!s.preview_message("remote").unwrap().pinned);
        s.apply(Update::Connection(Connection::Online));
        s.apply(Update::Connection(Connection::Offline));
        assert!(s.preview_message("remote").is_some());
        assert!(s.messages.is_empty());
    }

    #[test]
    fn stale_cached_body_is_refreshed_without_claiming_history_coverage() {
        let mut s = store();
        s.messages.push(convert(message(), "me"));
        s.search_results[0].content = "fresh body".into();
        assert!(s.search_message("remote").is_none());
        let ticket = s.begin_message_preview("elsewhere", "remote").unwrap();
        let mut fresh = message();
        fresh.content = Some("fresh body".into());
        assert!(s.finish_message_preview(&ticket, "remote", Some(fresh)));
        assert_eq!(s.search_message("remote").unwrap().content, "fresh body");
        assert_eq!(s.messages[0].content, "fresh body");
        assert!(s.timeline_ids.is_empty());
        assert!(s.pending_read_receipts().is_empty());
        let mut pin = message();
        pin.content = Some("fresh pinned body".into());
        s.apply(Update::PinnedMessages {
            ticket: None,
            channel_id: "elsewhere".into(),
            messages: vec![pin],
        });
        assert_eq!(s.pinned_in("elsewhere")[0].content, "fresh pinned body");
        assert_eq!(s.messages[0].content, "fresh pinned body");
        assert!(s.timeline_ids.is_empty());
    }

    #[test]
    fn pin_permissions_distinguish_server_roles_and_dm_participation() {
        let mut s = store();
        s.apply(Update::Channels(serde_json::from_value(serde_json::json!([{"id":"text","type":"text","name":"Chat"},{"id":"direct","type":"dm","name":"Direct"}])).unwrap()));
        assert!(!s.can_pin_in("text"));
        assert!(s.can_pin_in("direct"));
        assert!(!s.can_pin_in("missing"));
    }
}
