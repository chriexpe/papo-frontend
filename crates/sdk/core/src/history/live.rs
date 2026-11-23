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
        if channel.is_closed() {
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
        // A pin event names no channel, so a pin of a message no channel holds is left to the
        // next pin list fetch (`HistorySnapshot::pins`): it may be any channel's.
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
            let applied = persist_and_publish(
                session,
                &connection.cache,
                &channel,
                &mut state,
                &epoch,
                generation,
            )
            .await;
            // A seal that landed while this event was persisting stops only this channel.
            if applied.is_err() && channel.check().is_err() {
                continue;
            }
            applied?;
        }
        drop(state);
        if refresh_pins && channel.check().is_ok() {
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
            super::access::deny(
                session,
                &connection.client,
                &connection.cache,
                id,
                None,
                HistoryClosedReason::ChannelDeleted,
                generation,
            )
            .await?;
            return Ok(Vec::new());
        }
        // A quarantined channel keeps its model and cache current (nothing is delivered),
        // so a later grant resumes without a hole in its live edge.
        if let Some(channel) = session.history.live_channel(id).await {
            selected.push(channel);
        }
    } else {
        // The pinned backend omits channel IDs from reaction and pin events.
        // Only already-owned IDs may mutate a channel; in-flight pages replay the bounded journal.
        selected.extend(session.history.live_channels().await);
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
    session.inner.epochs.validate(generation)?;
    let id = Uuid::from_bytes(Model::id(
        message
            .channel_id
            .as_deref()
            .ok_or(ClientError::ResourceData)?,
    )?);
    let channel = session
        .inner
        .history
        .verified_channel(id)
        .await
        .map_err(|_| ClientError::ResourceData)?;
    channel.check()?;
    mutate(session, generation, &channel, |model| {
        model
            .receipt(message, if live { generation.get() } else { 0 })
            .map(|()| true)
    })
    .await
}

/// Settles a confirmed edit or deletion of this account's into history the way its socket
/// event would, so the outcome never waits for that event: the same channels (allowed or
/// quarantined), the same rules (an older edit never replaces a newer one, a deletion is
/// final). Settling again reaches the same state. A channel the session refuses, or one
/// sealed meanwhile, has no history to settle: that is done, never an error to retry.
pub(crate) async fn settle(
    session: &crate::Session,
    generation: Generation,
    channel_id: Uuid,
    event: &KnownEvent,
) -> Result<(), ClientError> {
    session.inner.epochs.validate(generation)?;
    let Some(channel) = session.inner.history.live_channel(channel_id).await else {
        return Ok(());
    };
    if channel.is_closed() {
        return Ok(());
    }
    let settled = mutate(session, generation, &channel, |model| {
        model.event(event, generation.get())
    })
    .await;
    if settled.is_err() && channel.check().is_err() {
        return Ok(());
    }
    settled
}

/// Settles a confirmed reaction change of this account's on `message` into history, with the
/// same channel rules as `settle`. Totals are left to the backend's reaction events.
pub(crate) async fn settle_own_reaction(
    session: &crate::Session,
    generation: Generation,
    channel_id: Uuid,
    message: Uuid,
    change: super::own_reactions::OwnReaction,
) -> Result<(), ClientError> {
    session.inner.epochs.validate(generation)?;
    let Some(channel) = session.inner.history.live_channel(channel_id).await else {
        return Ok(());
    };
    if channel.is_closed() {
        return Ok(());
    }
    let settled = mutate(session, generation, &channel, |model| {
        model.own_reaction(*message.as_bytes(), change)
    })
    .await;
    if settled.is_err() && channel.check().is_err() {
        return Ok(());
    }
    settled
}

/// Runs `apply` on `channel`'s model, then persists and publishes it when it reports a change.
async fn mutate(
    session: &crate::Session,
    generation: Generation,
    channel: &Arc<Channel>,
    apply: impl FnOnce(&mut Model) -> Result<bool, super::HistoryError>,
) -> Result<(), ClientError> {
    let epoch = session.inner.epochs.current();
    session.inner.epochs.validate(generation)?;
    let cache = session.client.inner.caches.open(session.account()).await?;
    let mut state = channel.state.lock().await;
    session.inner.epochs.validate(generation)?;
    state
        .ensure(&cache, *channel.id.as_bytes(), generation)
        .await?;
    if !apply(&mut state.model)? {
        return Ok(());
    }
    persist_and_publish(
        &session.inner,
        &cache,
        channel,
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

/// Read-only canonical evidence that history holds an edit of message `id` to exactly
/// `content` (an edited message with that text, or an edit waiting for its message), or that
/// the message is gone, so the edit has nothing left to settle. Never infers an edit from the
/// message merely existing.
pub(crate) async fn holds_edit(
    session: &crate::Session,
    channel: Uuid,
    id: Uuid,
    content: &str,
) -> bool {
    let channel = session
        .inner
        .history
        .channels
        .lock()
        .await
        .get(&channel)
        .cloned();
    let Some(channel) = channel else {
        return false;
    };
    let state = channel.state.lock().await;
    state.model.entries.get(id.as_bytes()).is_some_and(|entry| {
        let edited_message = entry.message.as_ref().is_some_and(|message| {
            message.edited_at.is_some() && message.content.as_deref() == Some(content)
        });
        let edit_waiting = entry
            .edit
            .as_ref()
            .is_some_and(|(_, edited)| edited == content);
        entry.deleted || edited_message || edit_waiting
    })
}
