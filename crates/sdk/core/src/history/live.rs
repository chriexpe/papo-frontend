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
    let unscoped_id = super::unscoped::message_id(event)?;
    if unscoped_id.is_some() {
        session.history.unscoped.record(generation, event);
    }
    let selected = select(session, connection, generation, event).await?;
    for channel in selected {
        if channel.check().is_err() {
            continue;
        }
        let mut state = channel.state.lock().await;
        session.epochs.validate(generation)?;
        if unscoped_id.is_some_and(|id| !state.model.entries.contains_key(&id)) {
            continue;
        }
        state
            .ensure(&connection.cache, *channel.id.as_bytes(), generation)
            .await?;
        if unscoped_id.is_some_and(|id| !state.model.entries.contains_key(&id)) {
            continue;
        }
        let refresh_pins = if let KnownEvent::MessagePin(event) = event {
            let id = Model::id(&event.message_id)?;
            event.is_pinned
                && state
                    .model
                    .entries
                    .get(&id)
                    .is_none_or(|entry| entry.message.is_none())
                && channel.is_open()
        } else {
            false
        };
        if state.model.event(event, generation.get())? {
            session.epochs.validate(generation)?;
            if let Err(error) = state
                .persist(&connection.cache, *channel.id.as_bytes(), &epoch)
                .await
            {
                state.reset();
                return Err(error.into());
            }
            session
                .epochs
                .apply(generation, || channel.publish(&state, generation))??;
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

async fn select(
    session: &SessionInner,
    connection: &ResourceConnection,
    generation: Generation,
    event: &KnownEvent,
) -> Result<Vec<Arc<Channel>>, ClientError> {
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
            return Ok(Vec::new());
        }
        if !session.history.verified.read().await.contains(&id) {
            return Ok(Vec::new());
        }
        let mut channels = session.history.channels.lock().await;
        let channel = Arc::clone(channels.entry(id).or_insert_with(|| {
            Arc::new(Channel::new(
                id,
                session.history.changes.clone(),
                Arc::clone(&session.history.counters),
            ))
        }));
        selected.push(channel);
    } else {
        // The pinned backend omits channel IDs from reaction and pin events.
        // Only already-owned IDs may mutate a channel; in-flight pages replay the bounded journal.
        selected.extend(session.history.channels.lock().await.values().cloned());
    }
    Ok(selected)
}

/// A confirmed creation is a live insertion, never pagination/coverage evidence.
/// Reuse normal event merge rules so prior deletes and newer edits still win.
pub(crate) async fn acknowledge(
    session: &crate::Session,
    rest: &papo_api::RestClient,
    generation: Generation,
    message: &papo_api::wire::Message,
) -> Result<(), ClientError> {
    session.inner.epochs.validate(generation)?;
    let cache = session.client.inner.caches.open(session.account()).await?;
    let connection = ResourceConnection {
        client: session.client.clone(),
        rest: rest.clone(),
        cache,
    };
    let event = papo_api::gateway::MessageEvent {
        id: message.id.clone().ok_or(ClientError::ResourceData)?,
        channel_id: message
            .channel_id
            .clone()
            .ok_or(ClientError::ResourceData)?,
        author_id: message.author_id.clone().ok_or(ClientError::ResourceData)?,
        content: message.content.clone().unwrap_or_default(),
        created_at: message
            .created_at
            .clone()
            .ok_or(ClientError::ResourceData)?,
        reply_to: message.reply_to.clone(),
        attachments: message.attachments.clone().unwrap_or_default(),
        webhook_id: message.webhook_id.clone(),
        external_author: message.external_author.clone(),
    };
    apply(
        &session.inner,
        &connection,
        generation,
        &Event::Known(KnownEvent::Message(Box::new(event))),
    )
    .await
}
