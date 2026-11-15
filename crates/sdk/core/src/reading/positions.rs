//! Coalescing coordinator and revocation fencing for durable saved reading positions.
//! Storage SQL lives in `catch_up_store`; catch-up intervals live in `catch_up`.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use papo_api::Uuid;
use tokio::sync::oneshot;

use super::position_publish::{clone_error, publish_positions};
use crate::{
    ClientError, Generation, HistoryClosedReason, HistoryError, ReadingError,
    catch_up::ReadingPosition, session::SessionInner,
};

struct PendingChannelWrite {
    generation: Generation,
    revocation_epoch: u64,
    position: Option<ReadingPosition>,
    waiters: Vec<oneshot::Sender<Result<(), ReadingError>>>,
}

#[derive(Default)]
struct State {
    flushing: bool,
    closed: bool,
    pending: BTreeMap<Uuid, PendingChannelWrite>,
    revocation_epochs: BTreeMap<Uuid, u64>,
    revoked: BTreeMap<Uuid, (Generation, HistoryClosedReason)>,
}

/// Coordinates rapid saved reading position writes so bursts coalesce onto one `synchronous=FULL`
/// transaction before callers receive acknowledgement, while discarding cleanly on revocation.
#[derive(Default)]
pub(crate) struct PositionCoordinator {
    state: Mutex<State>,
}

impl PositionCoordinator {
    pub fn check_writable(
        &self,
        channel: Uuid,
        generation: Generation,
        expected_epoch: u64,
    ) -> Option<HistoryClosedReason> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Some(HistoryClosedReason::AccountClosed);
        }
        let current_epoch = state.revocation_epochs.get(&channel).copied().unwrap_or(0);
        if current_epoch != expected_epoch {
            return Some(
                state
                    .revoked
                    .get(&channel)
                    .map_or(HistoryClosedReason::ChannelRemoved, |(_, r)| *r),
            );
        }
        if let Some((revoked_gen, reason)) = state.revoked.get(&channel)
            && *revoked_gen == generation
        {
            return Some(*reason);
        }
        None
    }

    pub fn check_current_channel(
        &self,
        channel: Uuid,
        generation: Generation,
    ) -> Option<HistoryClosedReason> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Some(HistoryClosedReason::AccountClosed);
        }
        state
            .revoked
            .get(&channel)
            .filter(|(revoked_gen, _)| *revoked_gen == generation)
            .map(|(_, reason)| *reason)
    }

    pub fn revoke(&self, channels: &[Uuid], generation: Generation, reason: HistoryClosedReason) {
        let mut waiters = Vec::new();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for &channel in channels {
                let epoch = state.revocation_epochs.entry(channel).or_insert(0);
                *epoch = epoch.saturating_add(1);
                state.revoked.insert(channel, (generation, reason));
                if let Some(entry) = state.pending.remove(&channel) {
                    waiters.extend(entry.waiters);
                }
            }
        }
        for waiter in waiters {
            let _ = waiter.send(Err(ReadingError::History(HistoryError::Closed(reason))));
        }
    }

    pub fn close(&self) {
        let mut waiters = Vec::new();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            for (_, entry) in std::mem::take(&mut state.pending) {
                waiters.extend(entry.waiters);
            }
        }
        for waiter in waiters {
            let _ = waiter.send(Err(ReadingError::Session(ClientError::Closed)));
        }
    }

    pub async fn save(
        &self,
        client: &crate::ClientHandle,
        session: &Arc<SessionInner>,
        channel: Uuid,
        generation: Generation,
        position: Option<ReadingPosition>,
    ) -> Result<(), ReadingError> {
        let (rx, start_flusher) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Err(ReadingError::Session(ClientError::Closed));
            }
            session.epochs.validate(generation)?;
            if let Some((revoked_gen, reason)) = state.revoked.get(&channel)
                && *revoked_gen == generation
            {
                return Err(ReadingError::History(HistoryError::Closed(*reason)));
            }
            let epoch = state.revocation_epochs.get(&channel).copied().unwrap_or(0);
            let (tx, rx) = oneshot::channel();
            let entry = state
                .pending
                .entry(channel)
                .or_insert_with(|| PendingChannelWrite {
                    generation,
                    revocation_epoch: epoch,
                    position,
                    waiters: Vec::new(),
                });
            if generation < entry.generation {
                return Err(ReadingError::Session(ClientError::Superseded));
            }
            // Superseded unstarted updates keep waiting for the later FULL commit.
            if generation > entry.generation {
                let expired = std::mem::take(&mut entry.waiters);
                for waiter in expired {
                    let _ = waiter.send(Err(ReadingError::Session(ClientError::Superseded)));
                }
                entry.generation = generation;
            }
            entry.revocation_epoch = epoch;
            entry.position = position;
            entry.waiters.push(tx);
            let start = !state.flushing;
            if start {
                state.flushing = true;
            }
            (rx, start)
        };
        if start_flusher {
            spawn_flusher(client.clone(), Arc::clone(session));
        }
        rx.await
            .map_err(|_| ReadingError::Session(ClientError::Closed))?
    }
}

fn spawn_flusher(client: crate::ClientHandle, session: Arc<SessionInner>) {
    let cancel = session.root.clone();
    let tasks = session.tasks.clone();
    tasks.spawn(async move {
        loop {
            let batch_map = {
                let mut state = session
                    .reading
                    .positions
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed || state.pending.is_empty() {
                    state.flushing = false;
                    return;
                }
                std::mem::take(&mut state.pending)
            };
            if cancel.is_cancelled() {
                notify_all(batch_map, &Err(ReadingError::Session(ClientError::Closed)));
                session
                    .reading
                    .positions
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .flushing = false;
                return;
            }
            flush_generation_batches(&client, &session, batch_map).await;
        }
    });
}

async fn flush_generation_batches(
    client: &crate::ClientHandle,
    session: &Arc<SessionInner>,
    batch_map: BTreeMap<Uuid, PendingChannelWrite>,
) {
    let mut by_gen: BTreeMap<Generation, Vec<(Uuid, PendingChannelWrite)>> = BTreeMap::new();
    for (channel, entry) in batch_map {
        by_gen
            .entry(entry.generation)
            .or_default()
            .push((channel, entry));
    }
    for (generation, entries) in by_gen {
        let mut to_commit = Vec::with_capacity(entries.len());
        let mut updates = Vec::with_capacity(entries.len());
        let mut waiters_by_channel = Vec::with_capacity(entries.len());
        for (channel, entry) in entries {
            to_commit.push((channel, entry.revocation_epoch, entry.position));
            updates.push((channel, entry.position));
            waiters_by_channel.push((channel, entry.waiters));
        }
        let result = client
            .inner
            .registry
            .commit_position_batch(Arc::clone(session), generation, to_commit)
            .await;
        match result {
            Ok(rejected) => {
                let mut accepted = Vec::new();
                let mut accepted_waiters = Vec::new();
                for (update, (channel, waiters)) in updates.into_iter().zip(waiters_by_channel) {
                    if let Some(reason) = rejected.get(&channel).copied() {
                        for waiter in waiters {
                            let _ = waiter
                                .send(Err(ReadingError::History(HistoryError::Closed(reason))));
                        }
                    } else if let Some(reason) = session
                        .reading
                        .positions
                        .revoked_reason(channel, generation)
                    {
                        for waiter in waiters {
                            let _ = waiter
                                .send(Err(ReadingError::History(HistoryError::Closed(reason))));
                        }
                    } else {
                        accepted.push(update);
                        accepted_waiters.push(waiters);
                    }
                }
                publish_positions(session, generation, accepted);
                for waiters in accepted_waiters {
                    for waiter in waiters {
                        let _ = waiter.send(Ok(()));
                    }
                }
            }
            Err(error) => {
                for (channel, waiters) in waiters_by_channel {
                    let per_channel_err = if let Some(reason) = session
                        .reading
                        .positions
                        .revoked_reason(channel, generation)
                    {
                        ReadingError::History(HistoryError::Closed(reason))
                    } else {
                        clone_error(&error)
                    };
                    for waiter in waiters {
                        let _ = waiter.send(Err(clone_error(&per_channel_err)));
                    }
                }
            }
        }
    }
}

impl PositionCoordinator {
    pub(super) fn revoked_reason(
        &self,
        channel: Uuid,
        generation: Generation,
    ) -> Option<HistoryClosedReason> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .revoked
            .get(&channel)
            .filter(|(g, _)| *g == generation)
            .map(|(_, r)| *r)
    }
}

fn notify_all(batch_map: BTreeMap<Uuid, PendingChannelWrite>, result: &Result<(), ReadingError>) {
    for (_, entry) in batch_map {
        for waiter in entry.waiters {
            let _ = waiter.send(result.as_ref().map(|&()| ()).map_err(clone_error));
        }
    }
}
