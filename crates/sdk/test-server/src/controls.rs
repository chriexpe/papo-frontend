//! Owns scriptable fixture resource mutations and typed live events; leaves HTTP dispatch and permission evaluation outside.

use papo_api::{
    Uuid,
    gateway::{
        AvatarUpdateEvent, ChannelCreateEvent, ChannelDeleteEvent, ChannelUpdateEvent,
        DMUpdateEvent, KnownEvent, RoleAddEvent, RoleRemoveEvent, UserJoinEvent,
    },
    wire::{Channel, DirectConversation, Role, Server, UserProfile, UserSummary},
};

use crate::{Error, TestServer, resources::CREATED_AT, server::State};

impl TestServer {
    /// Creates a deterministic text channel and broadcasts `channel_create` to current sockets.
    /// The authoritative snapshot changes even with no connected gateway.
    /// # Errors
    /// Returns [`Error::Configuration`] for an empty name or exhausted fixture counters.
    pub async fn create_channel(&self, name: &str) -> Result<Channel, Error> {
        if name.is_empty() {
            return Err(Error::Configuration);
        }
        let mut state = self.state.lock().await;
        let mut sequence = state.resources.next_channel_id;
        let id = loop {
            sequence = sequence
                .checked_add(1)
                .filter(|value| *value <= 0xffff_ffff_ffff)
                .ok_or(Error::Configuration)?;
            let id = format!("10000000-0000-4000-8000-{sequence:012x}");
            if !state.resources.channels.contains_key(&id) {
                break id;
            }
        };
        let position = state
            .resources
            .channels
            .values()
            .filter_map(|channel| channel.position)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(Error::Configuration)?;
        let channel = Channel {
            id: Some(id),
            name: Some(name.to_owned()),
            r#type: Some("text".into()),
            position: Some(position),
            permissions: Some(Vec::new()),
            created_at: Some(CREATED_AT.into()),
            ..Channel::default()
        };
        state.resources.next_channel_id = sequence;
        install_channel(&mut state, channel.clone())?;
        Ok(channel)
    }

    /// Renames an existing channel and broadcasts `channel_update`; returns false for a missing id or empty name.
    pub async fn rename_channel(&self, id: &str, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let mut state = self.state.lock().await;
        let Some(channel) = state.resources.channels.get_mut(id) else {
            return false;
        };
        channel.name = Some(name.to_owned());
        let event = channel_update(channel, id);
        emit(&state, None, event).is_ok()
    }

    /// Deletes a channel and its read marks and broadcasts `channel_delete`; returns false for a missing id.
    pub async fn delete_channel(&self, id: &str) -> bool {
        let mut state = self.state.lock().await;
        if state.resources.channels.remove(id).is_none() {
            return false;
        }
        state
            .resources
            .read_marks
            .retain(|(_, channel), _| channel != id);
        emit(
            &state,
            None,
            KnownEvent::ChannelDelete(Box::new(ChannelDeleteEvent {
                channel_id: id.to_owned(),
            })),
        )
        .is_ok()
    }

    /// Inserts or replaces a channel, including opaque permission overwrites.
    /// Emits create/update for channel metadata changes; overwrite-only changes have no backend event.
    /// # Errors
    /// Returns [`Error::Configuration`] for a missing/invalid UUID, empty name, missing type or position.
    pub async fn set_channel(&self, channel: Channel) -> Result<(), Error> {
        install_channel(&mut *self.state.lock().await, channel)
    }

    /// Replaces server discovery fields; counts are derived from fixture resources.
    /// The pinned backend has no server-info update event.
    pub async fn set_server_info(&self, server: Server) {
        self.state.lock().await.resources.server = Some(server);
    }

    /// Replaces role definitions. The pinned backend has no role-definition update event.
    pub async fn set_roles(&self, roles: Vec<Role>) {
        self.state.lock().await.resources.roles = roles;
    }

    /// Inserts/replaces a member profile and its lightweight member-list entry.
    /// Emits `user_join`, changed role assignments and changed avatars using pinned payloads.
    /// Other profile fields are fetched on demand; no synthetic profile-change event is emitted.
    /// # Errors
    /// Returns [`Error::Configuration`] for a missing/invalid UUID or missing username.
    pub async fn set_profile(&self, mut profile: UserProfile) -> Result<(), Error> {
        let id = profile
            .id
            .as_deref()
            .filter(|id| Uuid::parse_str(id).is_ok())
            .ok_or(Error::Configuration)?
            .to_owned();
        if profile.username.as_deref().is_none_or(str::is_empty) {
            return Err(Error::Configuration);
        }
        profile.created_at.get_or_insert_with(|| CREATED_AT.into());
        let mut state = self.state.lock().await;
        let previous = state.resources.profiles.insert(id.clone(), profile.clone());
        let Some(previous) = previous else {
            return emit(
                &state,
                None,
                KnownEvent::UserJoin(Box::new(UserJoinEvent { user_id: id })),
            );
        };
        for role in previous
            .roles
            .iter()
            .flatten()
            .filter_map(|role| role.id.as_ref())
        {
            if !profile
                .roles
                .iter()
                .flatten()
                .any(|current| current.id.as_ref() == Some(role))
            {
                emit(
                    &state,
                    None,
                    KnownEvent::RoleRemove(Box::new(RoleRemoveEvent {
                        user_id: id.clone(),
                        role_id: role.clone(),
                    })),
                )?;
            }
        }
        for role in profile
            .roles
            .iter()
            .flatten()
            .filter_map(|role| role.id.as_ref())
        {
            if !previous
                .roles
                .iter()
                .flatten()
                .any(|current| current.id.as_ref() == Some(role))
            {
                emit(
                    &state,
                    None,
                    KnownEvent::RoleAdd(Box::new(RoleAddEvent {
                        user_id: id.clone(),
                        role_id: role.clone(),
                    })),
                )?;
            }
        }
        if previous.avatar_blob != profile.avatar_blob
            || previous.avatar_format != profile.avatar_format
        {
            emit(
                &state,
                None,
                KnownEvent::AvatarUpdate(Box::new(AvatarUpdateEvent { user_id: id })),
            )?;
        }
        Ok(())
    }

    /// Removes a member and their on-demand profile without emitting a synthetic leave event.
    /// Returns false when the fixture had no such member.
    pub async fn remove_member(&self, user_id: &str) -> bool {
        self.state
            .lock()
            .await
            .resources
            .profiles
            .remove(user_id)
            .is_some()
    }

    /// Replaces one account's visible DMs and emits `dm_update` only to that account.
    /// Removing a DM from this snapshot has no synthetic deletion event.
    /// # Errors
    /// Returns [`Error::Configuration`] if a typed event cannot be encoded.
    pub async fn set_direct_conversations(
        &self,
        user_id: &str,
        dms: Vec<DirectConversation>,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        for dm in &dms {
            emit(
                &state,
                Some(user_id),
                KnownEvent::DMUpdate(Box::new(DMUpdateEvent { dm: dm.clone() })),
            )?;
        }
        state.resources.dms.insert(user_id.to_owned(), dms);
        Ok(())
    }

    /// Replaces one account's blocks. The pinned backend does not emit block-change events.
    pub async fn set_blocks(&self, user_id: &str, users: Vec<UserSummary>) {
        self.state
            .lock()
            .await
            .resources
            .blocks
            .insert(user_id.to_owned(), users);
    }

    /// Replaces one account's channel read mark, returned by `GET /channels` only to that account.
    /// The timestamp is retained verbatim; no history ordering or read-advance policy is evaluated.
    /// # Errors
    /// Returns [`Error::Configuration`] if the channel does not exist or the message UUID is invalid.
    pub async fn set_read_mark(
        &self,
        user_id: &str,
        channel_id: &str,
        message_id: &str,
        read_at: &str,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        if !state.resources.channels.contains_key(channel_id)
            || Uuid::parse_str(message_id).is_err()
        {
            return Err(Error::Configuration);
        }
        state.resources.read_marks.insert(
            (user_id.to_owned(), channel_id.to_owned()),
            (message_id.to_owned(), read_at.to_owned()),
        );
        Ok(())
    }

    /// Broadcasts a pinned typed event without modifying REST state, for duplicate/out-of-order scripts.
    /// # Errors
    /// Returns [`Error::Configuration`] if a typed event cannot be encoded.
    pub async fn emit_event(&self, event: KnownEvent) -> Result<(), Error> {
        emit(&*self.state.lock().await, None, event)
    }

    /// Sends a pinned event to one account without modifying REST state.
    /// # Errors
    /// Returns [`Error::Configuration`] if a typed event cannot be encoded.
    pub async fn emit_user_event(&self, user_id: &str, event: KnownEvent) -> Result<(), Error> {
        emit(&*self.state.lock().await, Some(user_id), event)
    }
}

fn install_channel(state: &mut State, channel: Channel) -> Result<(), Error> {
    let id = channel
        .id
        .as_deref()
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or(Error::Configuration)?
        .to_owned();
    if channel.name.as_deref().is_none_or(str::is_empty)
        || channel.r#type.is_none()
        || channel.position.is_none()
    {
        return Err(Error::Configuration);
    }
    let previous = state.resources.channels.insert(id.clone(), channel.clone());
    match previous {
        None => emit(
            state,
            None,
            KnownEvent::ChannelCreate(Box::new(ChannelCreateEvent {
                channel_id: id,
                name: channel.name.unwrap_or_default(),
                channel_type: channel.r#type.unwrap_or_default(),
                position: channel.position.unwrap_or_default(),
                topic: channel.topic,
            })),
        ),
        Some(previous)
            if (&previous.name, previous.position, &previous.topic)
                != (&channel.name, channel.position, &channel.topic) =>
        {
            emit(state, None, channel_update(&channel, &id))
        }
        Some(_) => Ok(()),
    }
}

fn channel_update(channel: &Channel, id: &str) -> KnownEvent {
    KnownEvent::ChannelUpdate(Box::new(ChannelUpdateEvent {
        channel_id: id.to_owned(),
        name: channel.name.clone().unwrap_or_default(),
        position: channel.position.unwrap_or_default(),
        topic: channel.topic.clone(),
    }))
}

fn emit(state: &State, recipient: Option<&str>, event: KnownEvent) -> Result<(), Error> {
    let payload = serde_json::to_value(event).map_err(|_| Error::Configuration)?;
    state.presence.emit(recipient, payload);
    Ok(())
}
