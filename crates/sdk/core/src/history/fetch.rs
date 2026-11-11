//! Generation-fenced page and pin requests; continuity checks and durable cache admission live elsewhere.

use std::sync::Arc;

use papo_api::{HistoryOrder, HistoryQuery};

use crate::{ClientError, Generation};

use super::{
    HistoryClosedReason, HistoryError, HistoryHandle, HistoryPage, HistorySource,
    channel::CacheConnection, merge::Ticket, model::Model, ordering::Point,
};

pub(super) async fn page(
    handle: &HistoryHandle,
    connection: &CacheConnection,
    ticket: Ticket,
) -> Result<HistoryPage, HistoryError> {
    let newest = ticket.cursor.is_none() && ticket.order == HistoryOrder::Desc;
    let (result, _) = server_page(handle, connection, ticket).await?;
    let epoch = handle.session.inner.epochs.current();
    if newest && epoch.generation == result.generation {
        let handle = handle.clone();
        let connection = connection.clone();
        let runtime = handle.session.client.inner.runtime.clone();
        let token = handle.session.inner.tasks.token();
        runtime.spawn(async move {
            let _token = token;
            let result = tokio::select! {
                biased;
                () = handle.session.inner.root.cancelled() => return,
                () = epoch.cancel.cancelled() => return,
                result = pins(&handle, &connection, epoch.generation) => result,
            };
            if let Err(error) = result
                && !matches!(
                    error,
                    HistoryError::Closed(_)
                        | HistoryError::Session(ClientError::Superseded)
                        | HistoryError::Cache(
                            crate::CacheError::Superseded | crate::CacheError::Closed
                        )
                )
            {
                if handle.check(epoch.generation).is_err() {
                    return;
                }
                if matches!(error, HistoryError::Cache(_)) {
                    let _ = handle
                        .session
                        .client
                        .inner
                        .caches
                        .invalidate(handle.account())
                        .await;
                }
                let _ = handle.session.reconnect();
            }
        });
    }
    Ok(result)
}

pub(super) async fn server_page(
    handle: &HistoryHandle,
    connection: &CacheConnection,
    ticket: Ticket,
) -> Result<(HistoryPage, Option<Point>), HistoryError> {
    let epoch = handle.session.inner.epochs.current();
    let generation = epoch.generation;
    if generation.get() != ticket.generation {
        return Err(ClientError::Superseded.into());
    }
    handle.check(generation)?;
    let cursor = ticket.cursor.as_ref().map(Point::cursor);
    let limit = u8::try_from(ticket.limit).map_err(|_| HistoryError::InvalidPage)?;
    let query = HistoryQuery {
        cursor: cursor.as_ref(),
        order: ticket.order,
        limit: Some(limit),
    };
    let response = handle
        .session
        .client
        .inner
        .request(connection.rest.messages(handle.channel.id, &query))
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return Err(rejection(handle, connection, generation, error).await),
    };
    handle.check(generation)?;
    if response.channel_id.as_deref().map(Model::id).transpose()? != Some(ticket.channel) {
        return Err(HistoryError::InvalidPage);
    }
    let messages = response.messages.ok_or(HistoryError::InvalidPage)?;
    let has_more = response.has_more.ok_or(HistoryError::InvalidPage)?;
    let points = ticket.validate(&messages, has_more)?;
    let oldest_requested = points.last().cloned();
    let mut state = handle.channel.state.lock().await;
    handle.check(generation)?;
    state
        .ensure(&connection.cache, ticket.channel, generation)
        .await?;
    state.model.merge_page(&ticket, &messages, has_more)?;
    if let Err(error) = state
        .persist(&connection.cache, ticket.channel, &epoch)
        .await
    {
        *state = super::persistence::State::default();
        return Err(error);
    }
    handle
        .session
        .inner
        .epochs
        .apply(generation, || handle.channel.publish(&state, generation))??;
    handle.check(generation)?;
    let messages = points
        .iter()
        .filter_map(|point| {
            let key = point.key().ok()?;
            state
                .model
                .entries
                .get(&key.id)?
                .message
                .clone()
                .map(Arc::new)
        })
        .collect();
    // The gap follows the page's own terminal boundary. A cursor that previously fell
    // inside a gap is filled by this response, not reported as still unknown.
    let gap = oldest_requested
        .as_ref()
        .map(|last| {
            state
                .model
                .page(Some(last), ticket.order, ticket.limit, generation.get())
        })
        .transpose()?
        .and_then(|page| page.gap);
    Ok((
        HistoryPage {
            generation,
            messages,
            next_cursor: oldest_requested.as_ref().map(Point::cursor),
            source: HistorySource::Server,
            stale: false,
            has_more,
            gap,
        },
        oldest_requested,
    ))
}

pub(super) async fn pins(
    handle: &HistoryHandle,
    connection: &CacheConnection,
    generation: Generation,
) -> Result<(), HistoryError> {
    let epoch = handle.session.inner.epochs.current();
    if epoch.generation != generation {
        return Err(ClientError::Superseded.into());
    }
    let _pins = handle.channel.pins_serial.lock().await;
    handle.check(generation)?;
    let ticket = {
        let mut state = handle.channel.state.lock().await;
        state
            .ensure(&connection.cache, *handle.channel.id.as_bytes(), generation)
            .await?;
        Ticket {
            channel: *handle.channel.id.as_bytes(),
            generation: generation.get(),
            revision: state.model.revision,
            cursor: None,
            order: HistoryOrder::Desc,
            limit: 100,
        }
    };
    let response = handle
        .session
        .client
        .inner
        .request(connection.rest.pinned_messages(handle.channel.id))
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return Err(rejection(handle, connection, generation, error).await),
    };
    handle.check(generation)?;
    if response.channel_id.as_deref().map(Model::id).transpose()? != Some(ticket.channel) {
        return Err(HistoryError::InvalidPage);
    }
    let messages = response.pinned.ok_or(HistoryError::InvalidPage)?;
    let mut state = handle.channel.state.lock().await;
    handle.check(generation)?;
    state.model.pins(&ticket, &messages)?;
    if let Err(error) = state
        .persist(&connection.cache, ticket.channel, &epoch)
        .await
    {
        *state = super::persistence::State::default();
        return Err(error);
    }
    handle
        .session
        .inner
        .epochs
        .apply(generation, || handle.channel.publish(&state, generation))??;
    Ok(())
}

async fn rejection(
    handle: &HistoryHandle,
    connection: &CacheConnection,
    generation: Generation,
    error: ClientError,
) -> HistoryError {
    if let Err(error) = handle.check(generation) {
        return error;
    }
    let status = match &error {
        ClientError::Api(
            papo_api::Error::Problem { status, .. } | papo_api::Error::Http { status },
        ) => *status,
        _ => return error.into(),
    };
    let reason = match status {
        403 => HistoryClosedReason::Forbidden,
        404 => HistoryClosedReason::NotFound,
        _ => return error.into(),
    };
    let _state = handle.channel.state.lock().await;
    // Loss of access ends capabilities even when persisting the revocation fails.
    handle.channel.close(reason);
    if let Err(error) = connection
        .cache
        .history_revoke(*handle.channel.id.as_bytes(), generation)
        .await
    {
        return error.into();
    }
    HistoryError::Closed(reason)
}
