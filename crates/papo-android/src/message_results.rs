//! Awaited result hydration and pin mutations through the shared API/reducer.
//! HTTP never holds the client lock and never selects/reads a conversation.
use super::*;
use crate::{
    account,
    bindings::{Backend, CoreError, NativeClient},
};

#[uniffi::export]
impl NativeClient {
    /// Register the visible pins sheet. Core's worker handles refresh requests;
    /// pin events carry no channel ID and may refer to unloaded messages.
    pub fn watch_pins(
        &self,
        owner: String,
        channel: String,
        enabled: bool,
    ) -> Result<(), CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        match backend.as_mut().ok_or("Client is closed".to_owned())? {
            Backend::Demo(c) => {
                if c.store.me != owner || c.store.channel(&channel).is_none() {
                    return Err("Account or channel changed".to_owned().into());
                }
            }
            Backend::Live(c) => {
                let scope = (owner, channel);
                if enabled {
                    if c.runtime.store.me != scope.0 || c.runtime.store.channel(&scope.1).is_none()
                    {
                        return Err("Account or channel changed".to_owned().into());
                    }
                    c.watched_pins = Some(scope);
                    c.watched_pins_dirty = false;
                } else if c.watched_pins.as_ref() == Some(&scope) {
                    c.watched_pins = None;
                    c.watched_pins_dirty = false;
                }
            }
        }
        Ok(())
    }

    pub fn load_pins(&self, owner: String, channel: String) -> Result<(), CoreError> {
        let ticket = {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            match backend.as_mut().ok_or("Client is closed".to_owned())? {
                Backend::Demo(c) => {
                    if c.store.me != owner || c.store.channel(&channel).is_none() {
                        return Err("Account or channel changed".to_owned().into());
                    }
                    let pins = c
                        .store
                        .pinned_in(&channel)
                        .iter()
                        .map(|m| m.id.clone())
                        .collect();
                    c.store.apply(Update::Pinned {
                        channel_id: channel.clone(),
                        ids: pins,
                    });
                    c.store.pinned_messages.entry(channel).or_default();
                    return Ok(());
                }
                Backend::Live(c) => {
                    if c.runtime.store.me != owner || c.runtime.store.channel(&channel).is_none() {
                        return Err("Account or channel changed".to_owned().into());
                    }
                    c.runtime.store.begin_load_pinned(&channel, false)
                }
            }
        };
        let Some(ticket) = ticket else {
            return Ok(());
        };
        let request = match self.scoped_api(&owner) {
            Ok(request) => request,
            Err(error) => {
                if let Ok(mut backend) = self.backend.lock()
                    && let Some(Backend::Live(c)) = backend.as_mut()
                {
                    c.runtime.store.apply(Update::PinnedFailed {
                        ticket,
                        message: error.to_string(),
                    });
                }
                return Err(error);
            }
        };
        match account::run(request.api.pinned(&channel)) {
            Ok(list) => self.finish_request(
                &request,
                vec![Update::PinnedMessages {
                    ticket: Some(ticket),
                    channel_id: channel,
                    messages: list.pinned,
                }],
            ),
            Err(error) => {
                let _ = self.finish_request(
                    &request,
                    vec![Update::PinnedFailed {
                        ticket,
                        message: error.to_string(),
                    }],
                );
                Err(error)
            }
        }
    }
    pub fn set_pin(
        &self,
        owner: String,
        channel: String,
        message_id: String,
        pinned: bool,
    ) -> Result<(), CoreError> {
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let backend = backend.as_mut().ok_or("Client is closed".to_owned())?;
            let store = match backend {
                Backend::Demo(c) => &mut c.store,
                Backend::Live(c) => &mut c.runtime.store,
            };
            if store.me != owner || !store.can_pin_in(&channel) {
                return Err(
                    "You do not have permission to manage pins in this conversation"
                        .to_owned()
                        .into(),
                );
            }
            if !store
                .preview_message(&message_id)
                .is_some_and(|m| m.channel_id == channel && !m.pending)
            {
                return Err("Message is no longer available".to_owned().into());
            }
            if let Backend::Demo(c) = backend {
                c.store.apply(Update::PinChanged {
                    channel_id: channel,
                    message_id,
                    pinned,
                });
                return Ok(());
            }
        }
        let request = self.scoped_api(&owner)?;
        account::run(async {
            if pinned {
                request
                    .api
                    .pin_message(&channel, &message_id)
                    .await
                    .map(|_| ())
            } else {
                match request.api.unpin_message(&channel, &message_id).await {
                    // Another participant may have already removed it. Confirm
                    // that desired state with the authoritative pin list.
                    Err(e @ papo_core::api::client::ApiError::NotFound) => {
                        let list = request.api.pinned(&channel).await?;
                        if list.pinned.iter().any(|m| m.id == message_id) {
                            Err(e)
                        } else {
                            Ok(())
                        }
                    }
                    other => other,
                }
            }
        })?;
        self.finish_request(
            &request,
            vec![Update::PinChanged {
                channel_id: channel.clone(),
                message_id,
                pinned,
            }],
        )?;
        // Refresh after acknowledgement so a concurrent participant's later
        // pin/unpin is represented by the authoritative list, with the core
        // journal protecting events that arrive during that refresh.
        let _ = self.load_pins(owner, channel);
        Ok(())
    }
    pub fn load_result_message(
        &self,
        owner: String,
        channel: String,
        message_id: String,
    ) -> Result<(), CoreError> {
        let (ticket, at) = {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            let store = match backend.as_mut().ok_or("Client is closed".to_owned())? {
                Backend::Demo(c) => &mut c.store,
                Backend::Live(c) => &mut c.runtime.store,
            };
            if store.me != owner {
                return Err("Account changed".to_owned().into());
            }
            let hit = store
                .search_results
                .iter()
                .find(|r| r.id == message_id && r.channel_id == channel)
                .ok_or("Search changed".to_owned())?;
            if store
                .search_message(&message_id)
                .is_some_and(|m| m.channel_id == channel)
            {
                return Ok(());
            }
            let at = hit
                .created_at
                .ok_or("The result has no message timestamp".to_owned())?;
            let ticket = store.begin_message_preview(&channel, &message_id);
            (ticket, at)
        };
        let Some(ticket) = ticket else {
            return Ok(());
        };
        let request = match self.scoped_api(&owner) {
            Ok(request) => request,
            Err(error) => {
                if let Ok(mut backend) = self.backend.lock()
                    && let Some(Backend::Live(c)) = backend.as_mut()
                {
                    c.runtime
                        .store
                        .finish_message_preview(&ticket, &message_id, None);
                }
                return Err(error);
            }
        };
        let result = account::run(request.api.message_context(&channel, &message_id, Some(at)));
        self.finish_request(&request, vec![])?;
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let Backend::Live(c) = backend.as_mut().ok_or("Client is closed".to_owned())? else {
            return Err("Account changed".to_owned().into());
        };
        if c.runtime.store.me != owner {
            return Err("Account changed".to_owned().into());
        }
        match result {
            Ok(context) => {
                let message = context.messages.into_iter().find(|m| m.id == message_id);
                if !c
                    .runtime
                    .store
                    .finish_message_preview(&ticket, &message_id, message)
                {
                    return Err("Search changed".to_owned().into());
                }
                Ok(())
            }
            Err(error) => {
                c.runtime
                    .store
                    .finish_message_preview(&ticket, &message_id, None);
                Err(error)
            }
        }
    }
}

impl Client {
    pub(super) fn observe_pin_update(&mut self, update: &Update) {
        if matches!(update, Update::Event(event) if matches!(event.as_ref(), papo_core::api::ws::Event::MessagePinned { .. }))
            || matches!(update, Update::Connection(Connection::Online))
        {
            self.watched_pins_dirty = true;
        }
    }

    pub(super) fn refresh_watched_pins(&mut self) {
        if !self.watched_pins_dirty {
            return;
        }
        let Some((owner, channel)) = self.watched_pins.as_ref() else {
            self.watched_pins_dirty = false;
            return;
        };
        if self.runtime.store.me != *owner || self.runtime.store.channel(channel).is_none() {
            self.watched_pins = None;
            self.watched_pins_dirty = false;
            return;
        }
        if self.runtime.store.connection != Connection::Online {
            return;
        }
        if let Some(ticket) = self.runtime.store.begin_load_pinned(channel, false) {
            self.watched_pins_dirty = false;
            self.runtime.net.send(Command::LoadPinned { ticket });
        }
        // If a refresh is already in flight, retain the dirty flag: its snapshot
        // might predate an event for an unknown message. Fetch once it settles.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_media_pins_and_search_cross_the_typed_boundary() {
        let native = NativeClient::demo();
        native
            .dispatch(Action::SelectChannel {
                channel_id: "media".into(),
            })
            .unwrap();
        let before = native.poll().unwrap();
        assert!(!before.messages.is_empty());
        native
            .set_pin("me".into(), "media".into(), "media-3".into(), true)
            .unwrap();
        native.load_pins("me".into(), "media".into()).unwrap();
        let pins = native.poll().unwrap();
        assert_eq!(pins.pinned_messages.len(), 1);
        assert!(!pins.pinned_messages[0].attachments.is_empty());
        native
            .dispatch(Action::Search {
                query: "em:#mídia".into(),
                more: false,
            })
            .unwrap();
        let results = native.poll().unwrap();
        assert!(!results.search_results.is_empty());
        assert!(results.search_results.iter().all(|r| r.message.is_some()));
    }
}
