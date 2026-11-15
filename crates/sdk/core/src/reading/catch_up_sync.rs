//! Storage hydration, channel revocation cleanup and snapshot reconciliation for catch-up and positions.
//! Live per-window reporting and deliberate clear ops live in `reading::catch_up_ops`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::Uuid;

use crate::{
    CatchUpSnapshot, ClientError, ClientHandle, Generation, HistoryClosedReason, HistorySnapshot,
    ReadingSnapshot, Session,
    catch_up::SnapshotIndex,
    reading::{counts, position_publish},
    session::SessionInner,
};

pub(crate) async fn hydrate_from_storage(
    client: &ClientHandle,
    session: &SessionInner,
) -> Result<(), ClientError> {
    let generation = session.epochs.current().generation;
    let stored_points = client
        .inner
        .registry
        .load_read_points(session.account)
        .await?;
    let (catch_up_map, position_map) = client
        .inner
        .registry
        .load_catch_up_and_positions(session.account)
        .await?;
    if !stored_points.is_empty() {
        let mut reading = ReadingSnapshot {
            generation,
            version: 0,
            channels: BTreeMap::new(),
        };
        for (channel, points) in stored_points {
            let mut state = super::ReadState {
                local_point: points.local,
                uploaded_point: points.uploaded.and_then(|u| u.point()),
                unread_count: super::UnreadCount::AtLeast(0),
                first_unread: None,
            };
            if state.point().is_none() {
                counts::unresolved(&mut state);
            }
            reading.channels.insert(channel, state);
        }
        session.reading.publish(session, reading)?;
    }
    if !catch_up_map.is_empty() {
        session.reading.publish_catch_up(
            session,
            CatchUpSnapshot {
                generation,
                version: 0,
                channels: catch_up_map,
            },
        )?;
    }
    if !position_map.is_empty() {
        let updates: Vec<_> = position_map
            .into_iter()
            .map(|(channel, pos)| (channel, Some(pos)))
            .collect();
        position_publish::publish_positions(session, generation, updates);
    }
    Ok(())
}

pub(crate) async fn revoke_channels(
    client: &ClientHandle,
    session: &SessionInner,
    channels: &[Uuid],
    generation: Generation,
    reason: HistoryClosedReason,
) -> Result<(), ClientError> {
    if channels.is_empty() {
        return Ok(());
    }
    session
        .reading
        .positions
        .revoke(channels, generation, reason);
    position_publish::remove_from_image(session, channels, generation);
    client
        .inner
        .registry
        .delete_saved_positions(session.account, channels.to_vec())
        .await?;
    Ok(())
}

pub(crate) async fn reconcile_removed_channels(
    client: &ClientHandle,
    session: &SessionInner,
    generation: Generation,
    visible: &BTreeSet<Uuid>,
) -> Result<(), ClientError> {
    let stale: Vec<Uuid> = session
        .reading
        .position_image
        .borrow()
        .channels
        .keys()
        .copied()
        .filter(|id| !visible.contains(id))
        .collect();
    if !stale.is_empty() {
        session
            .reading
            .positions
            .revoke(&stale, generation, HistoryClosedReason::ChannelRemoved);
        position_publish::remove_from_image(session, &stale, generation);
    }
    let deleted = client
        .inner
        .registry
        .retain_saved_positions(session.account, visible.clone())
        .await?;
    if !deleted.is_empty() {
        session
            .reading
            .positions
            .revoke(&deleted, generation, HistoryClosedReason::ChannelRemoved);
        position_publish::remove_from_image(session, &deleted, generation);
    }
    Ok(())
}

pub(crate) async fn reconcile_catch_up_snapshots(
    session: &Session,
    generation: Generation,
    snapshots: &[(Uuid, Arc<HistorySnapshot>)],
) -> Result<(), ClientError> {
    let mut catch_up_image = (**session.inner.reading.catch_up_image.borrow()).clone();
    let old_gen = catch_up_image.generation;
    let catch_up_version = catch_up_image.version;
    catch_up_image.generation = generation;
    let cached_keys = session
        .inner
        .reading
        .reconcile_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let mut updates = Vec::new();
    let mut verified_keys = Vec::new();
    for (channel, snapshot) in snapshots {
        if snapshot.generation != generation {
            continue;
        }
        let Some(current) = catch_up_image.channels.get(channel) else {
            continue;
        };
        if current.interval.is_none() {
            continue;
        }
        let (asc, asc_version) = session
            .inner
            .reading
            .asc_successors_with_version(generation, *channel);
        let key = (generation, snapshot.version, asc_version, catch_up_version);
        if cached_keys.get(channel) == Some(&key) {
            continue;
        }
        let index = SnapshotIndex::build(snapshot, session.account().user(), &asc)
            .map_err(|_| ClientError::ResourceData)?;
        let mut next = current.clone();
        if next.reconcile_snapshot(&index) {
            updates.push((*channel, next));
        } else {
            verified_keys.push((*channel, key));
        }
    }
    if !updates.is_empty() {
        session
            .client
            .inner
            .registry
            .commit_catch_up_batch(Arc::clone(&session.inner), generation, updates.clone())
            .await
            .map_err(ClientError::from)?;
        session.inner.epochs.validate(generation)?;
        for (channel, next) in updates {
            if next.seen_to.is_none() && next.interval.is_none() {
                catch_up_image.channels.remove(&channel);
            } else {
                catch_up_image.channels.insert(channel, next);
            }
        }
        session
            .inner
            .reading
            .publish_catch_up(&session.inner, catch_up_image)?;
    } else if old_gen != generation {
        session
            .inner
            .reading
            .publish_catch_up(&session.inner, catch_up_image)?;
    }
    if !verified_keys.is_empty() {
        let mut keys = session
            .inner
            .reading
            .reconcile_keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (channel, key) in verified_keys {
            keys.insert(channel, key);
        }
    }
    Ok(())
}
