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
    for (id, channel) in channels.iter() {
        if !visible.contains(id) {
            // Close before revoking: a reader that sees the revoked row must also see why.
            channel.close(HistoryClosedReason::ChannelRemoved);
            connection
                .cache
                .history_revoke(*id.as_bytes(), snapshot.generation)
                .await?;
        }
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
    let channels: Vec<_> = session
        .history
        .channels
        .lock()
        .await
        .values()
        .filter(|channel| channel.opens.load(Ordering::Relaxed) > 0 && channel.check().is_ok())
        .cloned()
        .collect();
    let cache_connection = CacheConnection {
        cache: Arc::clone(&connection.cache),
        rest: connection.rest.clone(),
        generation,
    };
    for channel in channels {
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
