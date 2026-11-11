//! Server-mark resolution and coalesced cache-driven unread refresh; fetches never acknowledge reads.

use super::{ReadPoint, ReadState, ReadingError, UnreadCount, counts, upload};
use crate::{
    ClientError, ResourceSnapshot, Session,
    read_points::{ServerReadPoint, UploadedPoint},
    reconcile::ResourceConnection,
    session::SessionInner,
};
use papo_api::{HistoryQuery, Uuid, wire::ChannelLastMessage};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
};

pub(crate) async fn resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
) -> Result<(), ClientError> {
    let handle = connection.client.session(session.account)?;
    if !std::ptr::eq(Arc::as_ptr(&handle.inner), session) {
        return Err(ClientError::Superseded);
    }
    start(&handle);
    let serial = session.reading.serial.lock().await;
    session.epochs.validate(snapshot.generation)?;
    let stored = connection
        .client
        .inner
        .registry
        .load_read_points(session.account)
        .await?;
    let mut image = (**session.reading.image.borrow()).clone();
    image.generation = snapshot.generation;
    let mut heads = BTreeMap::new();
    let mut unresolved = BTreeMap::new();
    let mut channels = BTreeMap::new();
    for Mark {
        channel,
        server,
        head,
    } in marks(snapshot)?
    {
        let previous = stored.get(&channel);
        let mut state = image.channels.remove(&channel).unwrap_or_default();
        state.local_point = state
            .local_point
            .max(previous.and_then(|points| points.local));
        state.uploaded_point = state.uploaded_point.max(
            previous
                .and_then(|points| points.uploaded.as_ref())
                .and_then(UploadedPoint::point),
        );
        if let Some(head) = head {
            heads.insert(channel, head);
        }
        if let Some(server) = server {
            let point = resolve(
                &handle,
                connection,
                snapshot.generation,
                channel,
                &server,
                &state,
                heads.get(&channel).copied(),
            )
            .await?;
            if point.is_none() {
                unresolved.insert(channel, server.clone());
            }
            let uploaded = UploadedPoint {
                server,
                created_at_us: point.map(|point| point.created_at_us),
                known_point: state.uploaded_point.max(point),
            };
            connection
                .client
                .inner
                .registry
                .save_uploaded_read(
                    Arc::clone(&handle.inner),
                    channel,
                    snapshot.generation,
                    uploaded,
                )
                .await?;
            state.uploaded_point = state.uploaded_point.max(point);
        }
        state.unread_count = UnreadCount::AtLeast(u64::from(
            heads
                .get(&channel)
                .is_some_and(|head| Some(*head) > state.point()),
        ));
        state.first_unread = None;
        if unresolved.contains_key(&channel) {
            counts::unresolved(&mut state);
        }
        channels.insert(channel, state);
    }
    image.channels = channels;
    install_resources(session, connection, image, heads, unresolved).await?;
    drop(serial);
    refresh(&handle).await?;
    let pending: Vec<_> = session
        .reading
        .image
        .borrow()
        .channels
        .iter()
        .filter_map(|(id, state)| state.pending().map(|_| *id))
        .collect();
    for channel in pending {
        upload::schedule(&handle, channel).await;
    }
    Ok(())
}

async fn install_resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    image: super::ReadingSnapshot,
    heads: BTreeMap<Uuid, ReadPoint>,
    unresolved: BTreeMap<Uuid, ServerReadPoint>,
) -> Result<(), ClientError> {
    {
        let mut stored_heads = session.reading.heads.lock().await;
        let mut stored_unresolved = session.reading.unresolved.lock().await;
        let mut stored_connection = session.reading.connection.lock().await;
        session.epochs.apply(image.generation, || {
            *stored_heads = heads;
            *stored_unresolved = unresolved;
            *stored_connection = Some((image.generation, connection.clone()));
        })?;
    }
    session.reading.publish(session, image)
}

struct Mark {
    channel: Uuid,
    server: Option<ServerReadPoint>,
    head: Option<ReadPoint>,
}

fn marks(snapshot: &ResourceSnapshot) -> Result<Vec<Mark>, ClientError> {
    snapshot
        .channels
        .iter()
        .map(|channel| {
            (
                channel.id.as_deref(),
                channel.last_read_message.as_deref(),
                channel.last_read_at.as_deref(),
                channel.last_message.as_ref(),
            )
        })
        .chain(snapshot.dms.iter().map(|dm| {
            (
                Some(dm.id.as_str()),
                dm.last_read_message.as_deref(),
                dm.last_read_at.as_deref(),
                dm.last_message.as_ref(),
            )
        }))
        .map(|(channel, message, recorded_at, head)| {
            let channel = channel
                .ok_or(ClientError::ResourceData)?
                .parse()
                .map_err(|_| ClientError::ResourceData)?;
            let server = message
                .map(|message| {
                    Ok::<_, ClientError>(ServerReadPoint {
                        message_id: message.parse().map_err(|_| ClientError::ResourceData)?,
                        recorded_at: recorded_at.map(str::to_owned),
                    })
                })
                .transpose()?;
            Ok(Mark {
                channel,
                server,
                head: head.map(head_point).transpose()?,
            })
        })
        .collect()
}

pub(super) fn head_point(head: &ChannelLastMessage) -> Result<ReadPoint, ClientError> {
    ReadPoint::parse(head.created_at.as_deref(), head.id.as_deref())
        .map_err(|_| ClientError::ResourceData)
}

/// A deleted anchor with a previously resolved key stays ordered. The pinned protocol
/// has no lookup-by-id endpoint: at most one newest-page read is attempted when absent.
/// Failure to find it is unknown, never a fabricated ordering from `recorded_at`.
pub(super) async fn resolve(
    session: &Session,
    connection: &ResourceConnection,
    generation: crate::Generation,
    channel: Uuid,
    server: &ServerReadPoint,
    state: &ReadState,
    head: Option<ReadPoint>,
) -> Result<Option<ReadPoint>, ClientError> {
    for point in [state.local_point, state.uploaded_point, head]
        .into_iter()
        .flatten()
    {
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    let history = session.history(channel).await?;
    let cached = history.subscribe().await?.snapshot;
    for message in &cached.messages {
        let point = ReadPoint::message(message).map_err(|_| ClientError::ResourceData)?;
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    let page = connection
        .client
        .inner
        .request(connection.rest.messages(
            channel,
            &HistoryQuery {
                limit: Some(100),
                ..HistoryQuery::default()
            },
        ))
        .await?;
    session.inner.epochs.validate(generation)?;
    if page
        .channel_id
        .as_deref()
        .and_then(|id| id.parse::<Uuid>().ok())
        != Some(channel)
    {
        return Err(ClientError::ResourceData);
    }
    for message in page.messages.unwrap_or_default() {
        if message
            .channel_id
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
            != Some(channel)
        {
            return Err(ClientError::ResourceData);
        }
        let point = ReadPoint::message(&message).map_err(|_| ClientError::ResourceData)?;
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    Ok(None)
}

fn start(session: &Session) {
    if session.inner.reading.started.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut changes = session.inner.history.subscribe_changes();
    let mut connection = session.subscribe_connection();
    let session = session.clone();
    let token = session.inner.tasks.token();
    let runtime = session.client.inner.runtime.clone();
    runtime.spawn(async move {
        let _token = token;
        loop {
            tokio::select! {
                biased;
                () = session.inner.root.cancelled() => return,
                result = changes.changed() => if result.is_err() { return; },
                result = connection.changed() => if result.is_err() { return; },
            }
            if let Err(ClientError::Closed) = refresh(&session).await {
                return;
            }
        }
    });
}

pub(super) async fn refresh(session: &Session) -> Result<(), ClientError> {
    let _serial = session.inner.reading.serial.lock().await;
    let generation = session.generation();
    session.inner.epochs.validate(generation)?;
    let snapshots = session.inner.history.snapshots().await;
    let heads = session.inner.reading.heads.lock().await;
    let mut unresolved = session.inner.reading.unresolved.lock().await;
    let mut image = (**session.inner.reading.image.borrow()).clone();
    let old_generation = image.generation;
    image.generation = generation;
    let previous = image.channels.clone();
    for (channel, snapshot) in snapshots {
        if snapshot.generation != generation {
            continue;
        }
        if let Some(state) = image.channels.get_mut(&channel) {
            if let Some(server) = unresolved.get(&channel) {
                let point = snapshot
                    .messages
                    .iter()
                    .map(|message| {
                        ReadPoint::message(message).map_err(|_| ClientError::ResourceData)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .find(|point| point.message_id == server.message_id);
                if let Some(point) = point {
                    session
                        .client
                        .inner
                        .registry
                        .save_uploaded_read(
                            Arc::clone(&session.inner),
                            channel,
                            generation,
                            UploadedPoint {
                                server: server.clone(),
                                created_at_us: Some(point.created_at_us),
                                known_point: state.uploaded_point.max(Some(point)),
                            },
                        )
                        .await?;
                    session.inner.epochs.validate(generation)?;
                    state.uploaded_point = state.uploaded_point.max(Some(point));
                    unresolved.remove(&channel);
                }
            }
            counts::update(state, &snapshot, heads.get(&channel).copied())
                .map_err(|_| ClientError::ResourceData)?;
            if unresolved.contains_key(&channel) {
                counts::unresolved(state);
            }
        }
    }
    if previous != image.channels || old_generation != generation {
        session.inner.reading.publish(&session.inner, image)?;
    }
    Ok(())
}

impl From<ReadingError> for ClientError {
    fn from(error: ReadingError) -> Self {
        match error {
            ReadingError::Session(error) => error,
            ReadingError::History(error) => error.into(),
            ReadingError::Storage(error) => error.into(),
            ReadingError::InvalidPoint | ReadingError::InvalidReport => Self::ResourceData,
        }
    }
}
