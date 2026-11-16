//! Storage hydration, channel revocation cleanup and snapshot reconciliation for catch-up and positions.
//! Live per-window reporting and deliberate clear ops live in `reading::catch_up_ops`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::Uuid;

use crate::{
    ClientError, ClientHandle, Generation, HistoryClosedReason, HistorySnapshot, ReadingSnapshot,
    Session,
    catch_up::{CatchUpHistory, CatchUpState},
    catch_up_store::CatchUpCommit,
    reading::{
        counts,
        position_publish::{self, PositionUpdate},
    },
    session::SessionInner,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReconcileKey {
    generation: Generation,
    snapshot_version: u64,
    catch_up_revision: Option<u64>,
}

struct ReconcileCandidate<'a> {
    channel: Uuid,
    snapshot: &'a Arc<HistorySnapshot>,
    current: CatchUpState,
    key: ReconcileKey,
}

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
    let loaded = client
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
                mention_count: super::UnreadCount::AtLeast(0),
                first_unread: None,
            };
            if state.point().is_none() {
                counts::unresolved(&mut state);
            }
            reading.channels.insert(channel, state);
        }
        session.reading.publish(session, reading)?;
    }
    session
        .reading
        .hydrate_catch_up(session, generation, loaded.catch_up)?;
    if !loaded.positions.is_empty() {
        let updates: Vec<_> = loaded
            .positions
            .into_iter()
            .map(|(channel, position)| PositionUpdate {
                channel,
                position: Some(position),
                expected_epoch: Some(0),
            })
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
    session.epochs.validate(generation)?;
    client
        .inner
        .registry
        .delete_channel_reading_state(session.account, channels.to_vec())
        .await?;
    // Publish removal after the synchronous=FULL delete commits and fences any newer grant.
    position_publish::remove_from_image(session, channels, generation);
    session
        .reading
        .remove_catch_up_channels(session, generation, channels);
    Ok(())
}

pub(crate) async fn reconcile_removed_channels(
    client: &ClientHandle,
    session: &SessionInner,
    generation: Generation,
    visible: &BTreeSet<Uuid>,
) -> Result<(), ClientError> {
    let mut removed_set = BTreeSet::new();
    for channel in session.reading.position_image.borrow().channels.keys() {
        if !visible.contains(channel) {
            removed_set.insert(*channel);
        }
    }
    for channel in session.reading.catch_up_image.borrow().channels.keys() {
        if !visible.contains(channel) {
            removed_set.insert(*channel);
        }
    }
    let stale: Vec<Uuid> = removed_set.iter().copied().collect();
    if !stale.is_empty() {
        session
            .reading
            .positions
            .revoke(&stale, generation, HistoryClosedReason::ChannelRemoved);
    }
    session.epochs.validate(generation)?;
    let deleted = client
        .inner
        .registry
        .retain_channel_reading_state(session.account, visible.clone())
        .await?;
    let new_deleted: Vec<Uuid> = deleted
        .iter()
        .copied()
        .filter(|channel| !removed_set.contains(channel))
        .collect();
    if !new_deleted.is_empty() {
        session.reading.positions.revoke(
            &new_deleted,
            generation,
            HistoryClosedReason::ChannelRemoved,
        );
    }
    removed_set.extend(deleted);
    if !removed_set.is_empty() {
        let removed: Vec<Uuid> = removed_set.into_iter().collect();
        position_publish::remove_from_image(session, &removed, generation);
        session
            .reading
            .remove_catch_up_channels(session, generation, &removed);
    }
    Ok(())
}

pub(crate) async fn reconcile_catch_up_snapshots(
    session: &Session,
    generation: Generation,
    snapshots: &[(Uuid, Arc<HistorySnapshot>)],
) -> Result<(), ClientError> {
    let (old_gen, candidates) = collect_reconcile_candidates(session, generation, snapshots);
    if old_gen == generation && candidates.is_empty() {
        return Ok(());
    }
    let mut commits = Vec::new();
    let mut updates = Vec::new();
    let mut verified_keys = Vec::new();
    for mut candidate in candidates {
        let index = CatchUpHistory::new(candidate.snapshot, session.account().user())
            .map_err(|_| ClientError::ResourceData)?;
        if candidate
            .current
            .reconcile_snapshot(&index)
            .map_err(|_| ClientError::ResourceData)?
        {
            commits.push(CatchUpCommit {
                channel: candidate.channel,
                state: candidate.current.clone(),
            });
            updates.push((candidate.channel, candidate.current));
        } else {
            verified_keys.push((candidate.channel, candidate.key));
        }
    }
    if !commits.is_empty() {
        session
            .client
            .inner
            .registry
            .commit_catch_up_batch(Arc::clone(&session.inner), generation, commits)
            .await?;
        session.inner.epochs.validate(generation)?;
    }
    session.inner.reading.apply_catch_up_updates(
        &session.inner,
        generation,
        updates,
        old_gen != generation,
    )?;
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

fn collect_reconcile_candidates<'a>(
    session: &Session,
    generation: Generation,
    snapshots: &'a [(Uuid, Arc<HistorySnapshot>)],
) -> (Generation, Vec<ReconcileCandidate<'a>>) {
    let catch_up_image = session.inner.reading.catch_up_image.borrow();
    let old_gen = catch_up_image.generation;
    let channels: Vec<Uuid> = snapshots
        .iter()
        .filter(|(_, snap)| snap.generation == generation)
        .map(|(ch, _)| *ch)
        .collect();
    let active_states: BTreeMap<Uuid, CatchUpState> = session
        .inner
        .reading
        .active_catch_up_intervals(&channels)
        .into_iter()
        .collect();
    let cached_keys = session
        .inner
        .reading
        .reconcile_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut candidates = Vec::new();
    for (channel, snapshot) in snapshots {
        if snapshot.generation != generation {
            continue;
        }
        let Some(current) = active_states.get(channel) else {
            continue;
        };
        let key = ReconcileKey {
            generation,
            snapshot_version: snapshot.version,
            catch_up_revision: catch_up_image.channels.get(channel).copied(),
        };
        if cached_keys.get(channel) == Some(&key) {
            continue;
        }
        candidates.push(ReconcileCandidate {
            channel: *channel,
            snapshot,
            current: current.clone(),
            key,
        });
    }
    (old_gen, candidates)
}
