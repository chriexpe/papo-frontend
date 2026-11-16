//! Server-mark resolution and coalesced cache-driven unread refresh; fetches never acknowledge reads.

use super::{UnreadCount, counts};
use crate::{ClientError, MessageKey, ResourceSnapshot, Session, read_points::UploadedPoint};
use papo_api::Uuid;
use std::sync::{Arc, atomic::Ordering};

pub(crate) use super::reconcile_resources::resources;

pub(super) fn start(session: &Session) {
    if session.inner.reading.started.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut changes = session.inner.history.subscribe_changes();
    let mut connection = session.subscribe_connection();
    let mut policy = session.inner.notifications.policy_changes.subscribe();
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
                result = policy.changed() => if result.is_err() { return; },
            }
            if let Err(ClientError::Closed) = refresh(&session).await {
                return;
            }
        }
    });
}

pub(crate) async fn refresh(session: &Session) -> Result<(), ClientError> {
    refresh_selected(session, None).await
}

pub(crate) async fn refresh_selected(
    session: &Session,
    selected: Option<&std::collections::BTreeSet<Uuid>>,
) -> Result<(), ClientError> {
    refresh_inner(session, selected, None).await
}

pub(crate) async fn refresh_with_resources(
    session: &Session,
    resources: &ResourceSnapshot,
    selected: Option<&std::collections::BTreeSet<Uuid>>,
) -> Result<(), ClientError> {
    refresh_inner(session, selected, Some(resources)).await
}

async fn refresh_inner(
    session: &Session,
    selected: Option<&std::collections::BTreeSet<Uuid>>,
    resources: Option<&ResourceSnapshot>,
) -> Result<(), ClientError> {
    let serial = session.inner.reading.serial.lock().await;
    let generation = session.generation();
    session.inner.epochs.validate(generation)?;
    // Loaded under `reading.serial` on purpose: `set_settings` commits new settings while holding
    // it, so a refresh never publishes counts computed from settings older than a commit it waited
    // for. The read goes through the reader pool, not the write path.
    let mut policy = counts::Policy::load(session)
        .await
        .map_err(|_| ClientError::ResourceData)?;
    if let Some(resources) = resources {
        policy = policy.with_resources(resources);
    }
    let snapshots = session.inner.history.snapshots().await;
    let heads = session.inner.reading.heads.lock().await;
    let mut unresolved = session.inner.reading.unresolved.lock().await;
    let mut image = (**session.inner.reading.image.borrow()).clone();
    let old_generation = image.generation;
    image.generation = generation;
    let previous = image.channels.clone();
    // Facts are kept only for channels whose history is still resident: a trimmed channel keeps
    // its last counts but must not keep its unread rows reachable through the cache.
    crate::notifications::classification::ClassificationCache::retain(
        &session.inner.notifications.classification,
        &snapshots
            .iter()
            .filter(|(_, snapshot)| snapshot.generation == generation)
            .map(|(channel, _)| *channel)
            .collect(),
    );
    for (channel, snapshot) in &snapshots {
        if selected.is_some_and(|selected| !selected.contains(channel))
            || snapshot.generation != generation
        {
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
                    session
                        .client
                        .inner
                        .registry
                        .save_uploaded_read(
                            Arc::clone(&session.inner),
                            *channel,
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
                    unresolved.remove(channel);
                }
            }
            policy
                .update(
                    session,
                    *channel,
                    state,
                    snapshot,
                    heads.get(channel).copied(),
                )
                .map_err(|_| ClientError::ResourceData)?;
            if unresolved.contains_key(channel) {
                counts::unresolved(state);
            }
        }
    }
    update_uncached_counts(&mut image, &policy, selected, &snapshots, generation);
    drop((heads, unresolved));
    if previous != image.channels || old_generation != generation {
        session.inner.reading.publish(&session.inner, image)?;
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

fn update_uncached_counts(
    image: &mut super::ReadingSnapshot,
    policy: &counts::Policy,
    selected: Option<&std::collections::BTreeSet<Uuid>>,
    snapshots: &[(Uuid, Arc<crate::HistorySnapshot>)],
    generation: crate::Generation,
) {
    for (channel, state) in &mut image.channels {
        if selected.is_none_or(|selected| selected.contains(channel)) {
            if !policy.eligible(*channel) {
                state.mention_count = UnreadCount::Exact(0);
            } else if !snapshots
                .iter()
                .any(|(id, snapshot)| id == channel && snapshot.generation == generation)
            {
                state.mention_count = UnreadCount::AtLeast(0);
            }
        }
    }
}
