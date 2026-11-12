//! Open-channel reconnect overlap and fresh-list authorization; live events use the existing reconcile journal.

use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::Ordering},
};

use papo_api::{HistoryOrder, Uuid};

use crate::{
    ClientError, Generation, ResourceSnapshot, Session, reconcile::ResourceConnection,
    session::SessionInner,
};

use super::{
    HistoryClosedReason, HistoryError, HistoryHandle,
    channel::{CacheConnection, Channel},
    merge::Ticket,
    model::Model,
};

pub(crate) async fn resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
) -> Result<(), ClientError> {
    session.epochs.validate(snapshot.generation)?;
    let mut visible = BTreeSet::new();
    for id in snapshot
        .channels
        .iter()
        .filter_map(|channel| channel.id.as_deref())
        .chain(snapshot.dms.iter().map(|channel| channel.id.as_str()))
    {
        visible.insert(Uuid::from_bytes(Model::id(id)?));
    }
    let store = &session.history;
    let mut channels = store.channels.lock().await;
    let previous = store.verified.read().await.clone();
    let revoked: BTreeSet<_> = previous
        .difference(&visible)
        .copied()
        .chain(channels.keys().filter(|id| !visible.contains(id)).copied())
        .collect();
    for id in revoked {
        if let Some(channel) = channels.get(&id) {
            channel.close(HistoryClosedReason::ChannelRemoved);
        }
        connection
            .cache
            .history_revoke(*id.as_bytes(), snapshot.generation)
            .await?;
    }
    // A re-grant creates a new handle identity; the revoked Arc is never revived.
    channels.retain(|_, channel| channel.check().is_ok());
    *store.verified.write().await = visible;
    *store.connection.write().await = Some(CacheConnection {
        cache: Arc::clone(&connection.cache),
        rest: connection.rest.clone(),
        generation: snapshot.generation,
    });
    Ok(())
}

pub(crate) async fn refresh(
    session: &SessionInner,
    connection: &ResourceConnection,
    generation: Generation,
) -> Result<(), ClientError> {
    let session_handle = connection.client.session(session.account)?;
    if !std::ptr::eq(Arc::as_ptr(&session_handle.inner), session) {
        return Err(ClientError::Superseded);
    }
    let unread = unread_channels(&session.reading).await;
    let verified = session.history.verified.read().await.clone();
    let mut selected = unread
        .intersection(&verified)
        .copied()
        .collect::<BTreeSet<_>>();
    {
        let channels = session.history.channels.lock().await;
        selected.extend(
            channels
                .iter()
                .filter(|(_, channel)| channel.is_open() && channel.check().is_ok())
                .map(|(id, _)| *id),
        );
    }
    let cache_connection = CacheConnection {
        cache: Arc::clone(&connection.cache),
        rest: connection.rest.clone(),
        generation,
    };
    for id in selected {
        let channel = {
            let mut channels = session.history.channels.lock().await;
            Arc::clone(channels.entry(id).or_insert_with(|| {
                Arc::new(Channel::new(
                    id,
                    session.history.changes.clone(),
                    Arc::clone(&session.history.counters),
                ))
            }))
        };
        match refresh_channel(&session_handle, channel, &cache_connection, generation).await {
            Ok(()) | Err(HistoryError::Closed(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

async fn refresh_channel(
    session: &Session,
    channel: Arc<Channel>,
    connection: &CacheConnection,
    generation: Generation,
) -> Result<(), HistoryError> {
    let old_newest = {
        let mut state = channel.state.lock().await;
        state
            .ensure(&connection.cache, *channel.id.as_bytes(), generation)
            .await?;
        state
            .model
            .chunks
            .last()
            .map(|chunk| {
                let first = chunk.first.key()?;
                let last = chunk.last.key()?;
                Ok::<_, HistoryError>(
                    state
                        .model
                        .order
                        .range(first..=last)
                        .map(|(_, id)| *id)
                        .collect::<BTreeSet<_>>(),
                )
            })
            .transpose()?
            .unwrap_or_default()
    };
    channel.opens.fetch_add(1, Ordering::Relaxed);
    let handle = HistoryHandle {
        session: session.clone(),
        channel,
    };
    let mut cursor = None;
    for attempt in 0..=3 {
        handle.check(generation)?;
        let ticket = {
            let state = handle.channel.state.lock().await;
            Ticket {
                channel: *handle.channel.id.as_bytes(),
                generation: generation.get(),
                revision: state.model.revision,
                cursor,
                order: HistoryOrder::Desc,
                limit: 100,
            }
        };
        let (page, oldest) = super::fetch::server_page(&handle, connection, ticket).await?;
        let overlap = oldest
            .as_ref()
            .map(|point| point.key().map(|key| old_newest.contains(&key.id)))
            .transpose()?
            .unwrap_or(false);
        if old_newest.is_empty() || overlap || !page.has_more || attempt == 3 {
            break;
        }
        cursor = oldest;
    }
    super::fetch::pins(&handle, connection, generation).await
}

/// Server heads and durable read points share message ordering; acknowledgement time is irrelevant.
pub(super) async fn unread_channels(reading: &crate::reading::Reading) -> BTreeSet<Uuid> {
    let heads = reading.heads.lock().await;
    let image = reading.image.borrow();
    heads
        .iter()
        .filter_map(|(channel, head)| {
            let point = image
                .channels
                .get(channel)
                .and_then(|state| state.local_point.max(state.uploaded_point));
            (Some(*head) > point).then_some(*channel)
        })
        .collect()
}
