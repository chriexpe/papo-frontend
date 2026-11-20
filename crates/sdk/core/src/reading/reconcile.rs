//! Server-mark resolution and coalesced cache-driven unread refresh; fetches never acknowledge reads.

use super::{
    ReadState, UnreadCount, counts,
    reconcile_marks::{Mark, marks, resolve},
    upload,
};
use crate::{
    ClientError, MessageKey, ResourceSnapshot, Session,
    history::access::ChannelAdmission,
    read_points::{ServerReadPoint, UploadedPoint},
    reconcile::ResourceConnection,
    session::SessionInner,
};
use papo_api::Uuid;
use std::{
    collections::{BTreeMap, BTreeSet},
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
    // History owns admission; reading state follows it rather than the raw channel list.
    let admitted = {
        let admissions = session.history.admission.read().await;
        Admitted {
            allowed: admissions.allowed(),
            quarantined: admissions.quarantined(),
        }
    };
    let (image, heads, unresolved) =
        build_channel_image(&handle, connection, snapshot, &admitted).await?;
    if snapshot.server.is_some() {
        // Quarantine keeps durable reading state for revalidation; only denial deletes it.
        let retained: BTreeSet<Uuid> = admitted
            .allowed
            .union(&admitted.quarantined)
            .copied()
            .collect();
        super::catch_up_sync::reconcile_removed_channels(
            &connection.client,
            session,
            snapshot.generation,
            &retained,
        )
        .await?;
    }
    install_resources(
        session,
        connection,
        image,
        heads,
        unresolved,
        snapshot.server.is_some(),
    )
    .await?;
    drop(serial);
    refresh(&handle).await?;
    resume_uploads(&handle).await;
    Ok(())
}

type ChannelImageParts = (
    super::ReadingSnapshot,
    BTreeMap<Uuid, MessageKey>,
    BTreeMap<Uuid, ServerReadPoint>,
);

struct Admitted {
    allowed: BTreeSet<Uuid>,
    quarantined: BTreeSet<Uuid>,
}

async fn build_channel_image(
    handle: &Session,
    connection: &ResourceConnection,
    snapshot: &ResourceSnapshot,
    admitted: &Admitted,
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
    for Mark {
        channel,
        server,
        head,
    } in marks(snapshot)
    {
        if admitted.quarantined.contains(&channel) {
            // Last-good state is kept as is; no server mark is resolved or saved for it.
            if let Some(state) = image.channels.remove(&channel) {
                channels.insert(channel, state);
            }
            continue;
        }
        if !admitted.allowed.contains(&channel) {
            continue;
        }
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
        save_admitted_mark(handle, channel, generation, uploaded).await?;
    }
    Ok(())
}

async fn save_admitted_mark(
    session: &Session,
    channel: Uuid,
    generation: crate::Generation,
    uploaded: UploadedPoint,
) -> Result<bool, ClientError> {
    let expected_epoch = {
        let admissions = session.inner.history.admission.read().await;
        if admissions.channel(channel) != Some(ChannelAdmission::Allowed) {
            return Ok(false);
        }
        session.inner.reading.positions.revocation_epoch(channel)
    };
    session
        .client
        .inner
        .registry
        .save_uploaded_read_at(
            Arc::clone(&session.inner),
            channel,
            generation,
            uploaded,
            expected_epoch,
        )
        .await
}

async fn install_resources(
    session: &SessionInner,
    connection: &ResourceConnection,
    mut image: super::ReadingSnapshot,
    mut heads: BTreeMap<Uuid, MessageKey>,
    mut unresolved: BTreeMap<Uuid, ServerReadPoint>,
    has_server: bool,
) -> Result<(), ClientError> {
    let mut stored_heads = session.reading.heads.lock().await;
    let mut stored_unresolved = session.reading.unresolved.lock().await;
    let mut stored_connection = session.reading.connection.lock().await;
    // A backend denial can finish while reading resolves a mark or waits for storage.
    // Hold R1 across the final fence update and publication, with all awaits completed.
    let admissions = session.history.admission.read().await;
    let last_good = session.reading.image.borrow().clone();
    image
        .channels
        .retain(|id, state| match admissions.channel(*id) {
            Some(ChannelAdmission::Allowed) => true,
            Some(ChannelAdmission::Quarantined) => {
                if let Some(previous) = last_good.channels.get(id) {
                    state.clone_from(previous);
                    true
                } else {
                    false
                }
            }
            Some(ChannelAdmission::Denied(_)) | None => false,
        });
    let allowed = admissions.allowed();
    heads.retain(|id, _| allowed.contains(id));
    unresolved.retain(|id, _| allowed.contains(id));
    session.epochs.apply(image.generation, || {
        if has_server {
            session
                .reading
                .positions
                .clear_revocations(&allowed, image.generation);
        }
        *stored_heads = heads;
        *stored_unresolved = unresolved;
        *stored_connection = Some((image.generation, connection.clone()));
        session.reading.publish_current(image)
    })?
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
        let mut maintenance = tokio::time::interval(std::time::Duration::from_secs(1));
        maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                () = session.inner.root.cancelled() => return,
                _ = maintenance.tick() => {},
                result = changes.changed() => if result.is_err() { return; },
                result = connection.changed() => if result.is_err() { return; },
            }
            if let Err(ClientError::Closed) = refresh(&session).await {
                return;
            }
        }
    });
}

pub(crate) async fn refresh(session: &Session) -> Result<(), ClientError> {
    let serial = session.inner.reading.serial.lock().await;
    let generation = session.generation();
    session.inner.epochs.validate(generation)?;
    let snapshots = session.inner.history.snapshots().await;
    let heads = session.inner.reading.heads.lock().await;
    let mut unresolved = session.inner.reading.unresolved.lock().await;
    let mut image = (**session.inner.reading.image.borrow()).clone();
    let old_generation = image.generation;
    image.generation = generation;
    let previous = image.channels.clone();
    for (channel, snapshot) in &snapshots {
        if snapshot.generation != generation {
            continue;
        }
        if let Some(state) = image.channels.get_mut(channel) {
            if let Some(server) = unresolved.get(channel) {
                let point = snapshot
                    .messages
                    .iter()
                    .map(|message| {
                        MessageKey::from_message(message).map_err(|_| ClientError::ResourceData)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .find(|point| point.message_id == server.message_id);
                if let Some(point) = point {
                    let mark = UploadedPoint {
                        server: server.clone(),
                        created_at_us: Some(point.created_at_us),
                        known_point: state.uploaded_point.max(Some(point)),
                    };
                    if !save_admitted_mark(session, *channel, generation, mark).await? {
                        continue;
                    }
                    session.inner.epochs.validate(generation)?;
                    state.uploaded_point = state.uploaded_point.max(Some(point));
                    unresolved.remove(channel);
                }
            }
            counts::update(state, snapshot, heads.get(channel).copied())
                .map_err(|_| ClientError::ResourceData)?;
            if unresolved.contains_key(channel) {
                counts::unresolved(state);
            }
        }
    }
    drop((heads, unresolved));
    if previous != image.channels || old_generation != generation {
        session.inner.reading.publish(&session.inner, image).await?;
    }
    super::catch_up_sync::reconcile_catch_up_snapshots(session, generation, &snapshots).await?;
    drop(serial);
    session
        .inner
        .history
        .trim(session.client.inner.config.history)
        .await;
    Ok(())
}

#[cfg(test)]
mod tests;
