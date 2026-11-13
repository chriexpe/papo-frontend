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
            persist_and_publish(
                session,
                &connection.cache,
                &channel,
                &mut state,
                &epoch,
                generation,
            )
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
        selected.push(session.history.channel(id).await);
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
    generation: Generation,
    message: &papo_api::wire::Message,
) -> Result<(), ClientError> {
    project_receipt(session, generation, message, true).await
}

/// Recover a saved receipt locally without asserting current live-edge or coverage evidence.
pub(crate) async fn restore_receipt(
    session: &crate::Session,
    message: &papo_api::wire::Message,
) -> Result<(), ClientError> {
    project_receipt(
        session,
        session.inner.epochs.current().generation,
        message,
        false,
    )
    .await
}

async fn project_receipt(
    session: &crate::Session,
    generation: Generation,
    message: &papo_api::wire::Message,
    live: bool,
) -> Result<(), ClientError> {
    let epoch = session.inner.epochs.current();
    session.inner.epochs.validate(generation)?;
    let id = Uuid::from_bytes(Model::id(
        message
            .channel_id
            .as_deref()
            .ok_or(ClientError::ResourceData)?,
    )?);
    if !session.inner.history.verified.read().await.contains(&id) {
        return Err(ClientError::ResourceData);
    }
    let cache = session.client.inner.caches.open(session.account()).await?;
    let channel = session.inner.history.channel(id).await;
    channel.check()?;
    let mut state = channel.state.lock().await;
    session.inner.epochs.validate(generation)?;
    state.ensure(&cache, *id.as_bytes(), generation).await?;
    state
        .model
        .receipt(message, if live { generation.get() } else { 0 })?;
    persist_and_publish(
        &session.inner,
        &cache,
        &channel,
        &mut state,
        &epoch,
        generation,
    )
    .await
}

async fn persist_and_publish(
    session: &SessionInner,
    cache: &crate::AccountCache,
    channel: &Channel,
    state: &mut super::persistence::State,
    epoch: &crate::generation::Epoch,
    generation: Generation,
) -> Result<(), ClientError> {
    if let Err(error) = state.persist(cache, *channel.id.as_bytes(), epoch).await {
        state.reset();
        return Err(error.into());
    }
    session
        .epochs
        .apply(generation, || channel.publish(state, generation))??;
    Ok(())
}

/// Read-only canonical evidence for an already confirmed outgoing ID.
pub(crate) async fn contains_receipt(session: &crate::Session, channel: Uuid, id: Uuid) -> bool {
    let channel = session
        .inner
        .history
        .channels
        .lock()
        .await
        .get(&channel)
        .cloned();
    if let Some(channel) = channel {
        let state = channel.state.lock().await;
        return state
            .model
            .entries
            .get(id.as_bytes())
            .is_some_and(|entry| entry.deleted || entry.timeline);
    }
    false
}
