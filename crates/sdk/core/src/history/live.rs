//! Journal-replayed history event routing and eviction; socket ownership remains in reconcile.

use std::sync::Arc;

use papo_api::{
    Uuid,
    gateway::{Event, KnownEvent},
};

use crate::{ClientError, Generation, reconcile::ResourceConnection, session::SessionInner};

use super::{HistoryClosedReason, channel::Channel, model::Model};

pub(crate) fn relevant(event: &Event) -> bool {
    matches!(
        event,
        Event::Known(
            KnownEvent::Message(_)
                | KnownEvent::MessageEdit(_)
                | KnownEvent::MessageDelete(_)
                | KnownEvent::MessageEmbedsUpdate(_)
                | KnownEvent::ReactUpdate(_)
                | KnownEvent::MessagePin(_)
                | KnownEvent::ChannelDelete(_)
        )
    )
}

pub(crate) async fn apply(
    session: &SessionInner,
    connection: &ResourceConnection,
    generation: Generation,
    event: &Event,
) -> Result<(), ClientError> {
    let epoch = session.epochs.current();
    session.epochs.validate(generation)?;
    let Event::Known(event) = event else {
        return Ok(());
    };
    let channel_id = match event {
        KnownEvent::Message(event) => Some(event.channel_id.as_str()),
        KnownEvent::MessageEdit(event) => Some(event.channel_id.as_str()),
        KnownEvent::MessageDelete(event) => Some(event.channel_id.as_str()),
        KnownEvent::MessageEmbedsUpdate(event) => Some(event.channel_id.as_str()),
        KnownEvent::ChannelDelete(event) => Some(event.channel_id.as_str()),
        _ => None,
    };
    let mut selected = Vec::new();
    if let Some(id) = channel_id {
        let id = Uuid::from_bytes(Model::id(id)?);
        if matches!(event, KnownEvent::ChannelDelete(_)) {
            // Close before revoking: a reader that sees the revoked row must also see why.
            if let Some(channel) = session.history.channels.lock().await.get(&id) {
                channel.close(HistoryClosedReason::ChannelDeleted);
            }
            connection
                .cache
                .history_revoke(*id.as_bytes(), generation)
                .await?;
            session.history.verified.write().await.remove(&id);
            return Ok(());
        }
        if !session.history.verified.read().await.contains(&id) {
            return Ok(());
        }
        let mut channels = session.history.channels.lock().await;
        let channel = Arc::clone(
            channels
                .entry(id)
                .or_insert_with(|| Arc::new(Channel::new(id))),
        );
        selected.push(channel);
    } else {
        // The pinned backend omits channel IDs from reaction and pin events.
        // Unloaded channels stay stale and are revalidated by their next server page.
        selected.extend(session.history.channels.lock().await.values().cloned());
    }
    for channel in selected {
        if channel.check().is_err() {
            continue;
        }
        let mut state = channel.state.lock().await;
        session.epochs.validate(generation)?;
        state
            .ensure(&connection.cache, *channel.id.as_bytes(), generation)
            .await?;
        let refresh_pins = if let KnownEvent::MessagePin(event) = event {
            let id = Model::id(&event.message_id)?;
            event.is_pinned
                && state
                    .model
                    .entries
                    .get(&id)
                    .is_none_or(|entry| entry.message.is_none())
                && channel.opens.load(std::sync::atomic::Ordering::Relaxed) > 0
        } else {
            false
        };
        if state.model.event(event, generation.get())? {
            session.epochs.validate(generation)?;
            state
                .persist(&connection.cache, *channel.id.as_bytes(), &epoch)
                .await?;
        }
        drop(state);
        if refresh_pins {
            let session_handle = connection.client.session(session.account)?;
            if !std::ptr::eq(Arc::as_ptr(&session_handle.inner), session) {
                return Err(ClientError::Superseded);
            }
            channel
                .opens
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let handle = super::HistoryHandle {
                session: session_handle,
                channel: Arc::clone(&channel),
            };
            let current = super::channel::CacheConnection {
                cache: Arc::clone(&connection.cache),
                rest: connection.rest.clone(),
                generation,
            };
            super::fetch::pins(&handle, &current, generation).await?;
        }
    }
    Ok(())
}
