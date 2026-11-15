//! Coalescing coordinator and revocation fencing for durable saved reading positions.
//! Storage SQL lives in `catch_up_position_store`; catch-up intervals live in `catch_up`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use papo_api::Uuid;
use tokio::sync::oneshot;

use super::position_publish::{PositionUpdate, publish_positions};
use crate::{
    ClientError, Generation, HistoryClosedReason, HistoryError, ReadingError,
    catch_up::ReadingPosition, catch_up_position_store::PendingPositionCommit,
    session::SessionInner,
};

struct PendingChannelWrite {
    generation: Generation,
    revocation_epoch: u64,
    position: Option<ReadingPosition>,
    waiters: Vec<oneshot::Sender<Result<(), ReadingError>>>,
}

#[derive(Clone, Copy)]
struct ChannelRevocation {
    generation: Generation,
    reason: HistoryClosedReason,
}

#[derive(Clone, Copy)]
struct RevocationEpoch {
    epoch: u64,
    reason: HistoryClosedReason,
}

#[derive(Default)]
struct State {
    flushing: bool,
    closed: bool,
    pending: BTreeMap<Uuid, PendingChannelWrite>,
    revocation_epochs: BTreeMap<Uuid, RevocationEpoch>,
    revoked: BTreeMap<Uuid, ChannelRevocation>,
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
        let current = state.revocation_epochs.get(&channel).copied();
        let current_epoch = current.map_or(0, |entry| entry.epoch);
        if current_epoch != expected_epoch {
            return Some(
                state
                    .revoked
                    .get(&channel)
                    .map(|entry| entry.reason)
                    .or_else(|| current.map(|entry| entry.reason))
                    .unwrap_or(HistoryClosedReason::ChannelRemoved),
            );
        }
        if let Some(entry) = state.revoked.get(&channel)
            && entry.generation == generation
        {
            return Some(entry.reason);
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
            .filter(|entry| entry.generation == generation)
            .map(|entry| entry.reason)
    }

    pub fn revoke(&self, channels: &[Uuid], generation: Generation, reason: HistoryClosedReason) {
        let mut waiters = Vec::new();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for &channel in channels {
                let next_epoch = state
                    .revocation_epochs
                    .get(&channel)
                    .map_or(0, |entry| entry.epoch)
                    .saturating_add(1);
                state.revocation_epochs.insert(
                    channel,
                    RevocationEpoch {
                        epoch: next_epoch,
                        reason,
                    },
                );
                state
                    .revoked
                    .insert(channel, ChannelRevocation { generation, reason });
                if let Some(entry) = state.pending.remove(&channel) {
                    waiters.extend(entry.waiters);
                }
            }
        }
        if !waiters.is_empty() {
            fail_waiters(waiters, || {
                ReadingError::History(HistoryError::Closed(reason))
            });
        }
    }

    pub fn clear_revocations(&self, visible: &BTreeSet<Uuid>, generation: Generation) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .revoked
            .retain(|channel, entry| entry.generation != generation || !visible.contains(channel));
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
        if !waiters.is_empty() {
            fail_waiters(waiters, || ReadingError::Session(ClientError::Closed));
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
        // Validate generation before locking `self.state` to preserve `epochs -> state` order.
        session.epochs.validate(generation)?;
        let (rx, start_flusher) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return Err(ReadingError::Session(ClientError::Closed));
            }
            if let Some(entry) = state.revoked.get(&channel)
                && entry.generation == generation
            {
                return Err(ReadingError::History(HistoryError::Closed(entry.reason)));
            }
            let epoch = state
                .revocation_epochs
                .get(&channel)
                .map_or(0, |entry| entry.epoch);
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
                if !expired.is_empty() {
                    fail_waiters(expired, || ReadingError::Session(ClientError::Superseded));
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
                fail_all_waiters(batch_map, || ReadingError::Session(ClientError::Closed));
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
            to_commit.push(PendingPositionCommit {
                channel,
                revocation_epoch: entry.revocation_epoch,
                position: entry.position,
            });
            updates.push(PositionUpdate {
                channel,
                position: entry.position,
                expected_epoch: Some(entry.revocation_epoch),
            });
            waiters_by_channel.push((channel, entry.revocation_epoch, entry.waiters));
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
                for (update, (channel, revocation_epoch, waiters)) in
                    updates.into_iter().zip(waiters_by_channel)
                {
                    if let Some(reason) = rejected.get(&channel).copied() {
                        fail_waiters(waiters, || {
                            ReadingError::History(HistoryError::Closed(reason))
                        });
                    } else if let Some(reason) = session.reading.positions.check_writable(
                        channel,
                        generation,
                        revocation_epoch,
                    ) {
                        fail_waiters(waiters, || {
                            ReadingError::History(HistoryError::Closed(reason))
                        });
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
                for (channel, revocation_epoch, waiters) in waiters_by_channel {
                    if let Some(reason) = session.reading.positions.check_writable(
                        channel,
                        generation,
                        revocation_epoch,
                    ) {
                        fail_waiters(waiters, || {
                            ReadingError::History(HistoryError::Closed(reason))
                        });
                    } else {
                        fail_waiters(waiters, || error.clone().into());
                    }
                }
            }
        }
    }
}

fn fail_waiters(
    waiters: Vec<oneshot::Sender<Result<(), ReadingError>>>,
    mut make_error: impl FnMut() -> ReadingError,
) {
    for waiter in waiters {
        let _ = waiter.send(Err(make_error()));
    }
}

fn fail_all_waiters(
    batch_map: BTreeMap<Uuid, PendingChannelWrite>,
    mut make_error: impl FnMut() -> ReadingError,
) {
    for (_, entry) in batch_map {
        fail_waiters(entry.waiters, &mut make_error);
    }
}

#[cfg(test)]
#[path = "positions_tests.rs"]
mod tests;
