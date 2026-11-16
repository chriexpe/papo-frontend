//! Resource replacement updates durable read marks and recounts against the incoming policy image.

use super::{
    ReadState, UnreadCount, counts,
    reconcile_marks::{Mark, marks, resolve},
    upload,
};
use crate::{
    ClientError, MessageKey, ResourceSnapshot, Session,
    read_points::{ServerReadPoint, UploadedPoint},
    reconcile::ResourceConnection,
    session::SessionInner,
};
use papo_api::Uuid;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) async fn resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
) -> Result<(), ClientError> {
    let handle = connection.client.session(session.account)?;
    if !std::ptr::eq(Arc::as_ptr(&handle.inner), session) {
        return Err(ClientError::Superseded);
    }
    super::reconcile::start(&handle);
    let serial = session.reading.serial.lock().await;
    session.epochs.validate(snapshot.generation)?;
    let (image, heads, unresolved) = build_channel_image(&handle, connection, snapshot).await?;
    let previous = session.reading.image.borrow().clone();
    let old_heads = session.reading.heads.lock().await.clone();
    let old_resources = session.resources.borrow().clone();
    let mode = |resources: &ResourceSnapshot, channel: Uuid| {
        resources
            .channels
            .iter()
            .find(|item| item.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
            .and_then(|item| item.notification_settings.clone())
    };
    let broadcasts = session
        .notifications
        .classification
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .broadcast_channels();
    let dm = |resources: &ResourceSnapshot, channel: Uuid| {
        resources
            .dms
            .iter()
            .any(|dm| dm.id.parse().ok() == Some(channel))
    };
    let affected = image
        .channels
        .iter()
        .filter_map(|(channel, state)| {
            (previous.generation != image.generation
                || previous.channels.get(channel) != Some(state)
                || old_heads.get(channel) != heads.get(channel)
                || mode(&old_resources, *channel) != mode(snapshot, *channel)
                || dm(&old_resources, *channel) != dm(snapshot, *channel)
                || broadcasts.contains(channel))
            .then_some(*channel)
        })
        .collect();
    if snapshot.server.is_some() {
        let visible: std::collections::BTreeSet<Uuid> = image.channels.keys().copied().collect();
        super::catch_up_sync::reconcile_removed_channels(
            &connection.client,
            session,
            snapshot.generation,
            &visible,
        )
        .await?;
        session
            .reading
            .positions
            .clear_revocations(&visible, snapshot.generation);
    }
    install_resources(session, connection, image, heads, unresolved).await?;
    drop(serial);
    super::reconcile::refresh_with_resources(&handle, snapshot, Some(&affected)).await?;
    resume_uploads(&handle).await;
    Ok(())
}

type ChannelImageParts = (
    super::ReadingSnapshot,
    BTreeMap<Uuid, MessageKey>,
    BTreeMap<Uuid, ServerReadPoint>,
);

async fn build_channel_image(
    handle: &Session,
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
) -> Result<ChannelImageParts, ClientError> {
    let session = &handle.inner;
    let stored = connection
        .client
        .inner
        .registry
        .load_read_points(session.account)
        .await?;
    let mut image = (**session.reading.image.borrow()).clone();
    let same_generation = image.generation == snapshot.generation;
    let previously_unresolved = session.reading.unresolved.lock().await.clone();
    image.generation = snapshot.generation;
    let mut heads = BTreeMap::new();
    let mut unresolved = BTreeMap::new();
    let mut channels = BTreeMap::new();
    let cached: std::collections::BTreeSet<_> = handle
        .inner
        .history
        .snapshots()
        .await
        .into_iter()
        .map(|(channel, _)| channel)
        .collect();
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
            let saved = previous.and_then(|points| points.uploaded.as_ref());
            let point = if let Some(point) = saved
                .filter(|saved| saved.server.message_id == server.message_id)
                .and_then(|saved| saved.created_at_us)
            {
                Some(MessageKey {
                    created_at_us: point,
                    message_id: server.message_id,
                })
            } else if same_generation && previously_unresolved.get(&channel) == Some(&server) {
                None
            } else {
                resolve(
                    handle,
                    connection,
                    snapshot.generation,
                    channel,
                    &server,
                    &state,
                    heads.get(&channel).copied(),
                )
                .await?
            };
            if point.is_none() {
                unresolved.insert(channel, server.clone());
            }
            save_resolved(
                handle,
                snapshot.generation,
                channel,
                server,
                point,
                &state,
                saved,
            )
            .await?;
            state.uploaded_point = state.uploaded_point.max(point);
        }
        if !same_generation || !cached.contains(&channel) {
            state.unread_count = UnreadCount::AtLeast(u64::from(
                heads
                    .get(&channel)
                    .is_some_and(|head| Some(*head) > state.point()),
            ));
            state.mention_count = UnreadCount::AtLeast(0);
            state.first_unread = None;
        }
        if unresolved.contains_key(&channel) {
            counts::unresolved(&mut state);
        }
        channels.insert(channel, state);
    }
    image.channels = channels;
    Ok((image, heads, unresolved))
}

async fn resume_uploads(handle: &Session) {
    let pending: Vec<_> = handle
        .inner
        .reading
        .image
        .borrow()
        .channels
        .iter()
        .filter_map(|(id, state)| state.pending().map(|_| *id))
        .collect();
    for channel in pending {
        upload::schedule(handle, channel).await;
    }
}

async fn save_resolved(
    handle: &Session,
    generation: crate::Generation,
    channel: Uuid,
    server: ServerReadPoint,
    point: Option<MessageKey>,
    state: &ReadState,
    saved: Option<&UploadedPoint>,
) -> Result<(), ClientError> {
    let uploaded = UploadedPoint {
        server,
        created_at_us: point.map(|point| point.created_at_us),
        known_point: state.uploaded_point.max(point),
    };
    if saved != Some(&uploaded) {
        handle
            .client
            .inner
            .registry
            .save_uploaded_read(Arc::clone(&handle.inner), channel, generation, uploaded)
            .await?;
    }
    Ok(())
}

async fn install_resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    image: super::ReadingSnapshot,
    heads: BTreeMap<Uuid, MessageKey>,
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
    if session.reading.image.borrow().generation != image.generation
        || session.reading.image.borrow().channels != image.channels
    {
        session.reading.publish(session, image)?;
    }
    Ok(())
}
