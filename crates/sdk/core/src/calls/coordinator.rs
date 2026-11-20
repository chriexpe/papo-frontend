//! Session-owned call coordinator core: join admission, attempt termination and effect dispatch.
//! Attempt state lives in `attempt.rs`, session lifecycle reactions in `lifecycle.rs`,
//! gateway event handling in `gateway_events.rs`, media reports in `media_reports.rs`,
//! handle controls in `controls.rs`, and async startup/watchdogs in `startup.rs`.

use crate::{
    AccountId, Generation, ResourceSnapshot, Uuid,
    observable::StateCell,
    permissions::{VoiceAccess, voice_access},
};
use papo_api::gateway::Command;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(crate) use super::attempt::{ActiveAttempt, ActivePublish, CoordinatorState, SideEffects};
use super::{
    fifo::{EnqueueError, QueuedVoiceCommand, VoiceCommandReceiver},
    lease::ClientCallLease,
    media_port::{MediaCommand, MediaEngineFactory},
    model::{
        CallAttemptId, CallConfig, CallEndReason, CallError, CallPhase, CallSnapshot, RoomsSnapshot,
    },
    reducer::{ActivePeerRef, VoiceReducer},
    startup::{self, IceFetcher},
};

/// Session-owned call controller shared by gateway receive, `AccountSocket`, handles and UI.
pub(crate) struct SessionCalls {
    pub account: AccountId,
    pub config: CallConfig,
    pub client_lease: Arc<ClientCallLease>,
    pub rooms_cell: Arc<StateCell<RoomsSnapshot>>,
    pub active_cell: Arc<StateCell<Option<CallSnapshot>>>,
    pub state: Mutex<CoordinatorState>,
    pub closed: AtomicU64,
}

impl SessionCalls {
    /// Creates a new call coordinator bound to `account` and the client-wide call lease.
    #[must_use]
    pub(crate) fn new(
        account: AccountId,
        generation: Generation,
        config: CallConfig,
        client_lease: Arc<ClientCallLease>,
    ) -> Arc<Self> {
        let initial_resources = Arc::new(ResourceSnapshot::empty(generation));
        Arc::new(Self {
            account,
            config,
            client_lease,
            rooms_cell: StateCell::new(RoomsSnapshot::empty(generation)),
            active_cell: StateCell::new(None),
            state: Mutex::new(CoordinatorState {
                generation,
                online: false,
                resources: initial_resources,
                reducer: VoiceReducer::new(generation),
                rooms_version: 0,
                active: None,
                draining_attempts: BTreeSet::new(),
                abandoned_joins: BTreeMap::new(),
                command_sender: None,
                next_host_lease_id: 1,
            }),
            closed: AtomicU64::new(0),
        })
    }

    /// Creates a fresh bounded voice FIFO attached to `generation` for a new `AccountSocket`.
    pub(crate) fn attach_socket(&self, generation: Generation) -> VoiceCommandReceiver {
        let (tx, rx) = super::fifo::channel(self.config.command_queue_capacity);
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.command_sender = Some((generation, tx));
        rx
    }

    /// Checks whether `AccountSocket` should transmit a queued voice command on `generation`.
    #[must_use]
    pub(crate) fn should_send(&self, item: &QueuedVoiceCommand, socket_gen: Generation) -> bool {
        if item.generation != socket_gen {
            return false;
        }
        if item.allow_after_end {
            return true;
        }
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.active.as_ref().is_some_and(|active| {
            active.attempt == item.attempt
                && active.generation == socket_gen
                && active.phase.is_active()
        })
    }

    /// Returns a cloned snapshot cell for the currently active attempt, if any.
    #[must_use]
    pub(crate) fn active_attempt_handle_parts(
        &self,
    ) -> Option<(CallAttemptId, Arc<StateCell<CallSnapshot>>)> {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .active
            .as_ref()
            .filter(|active| active.phase.is_active())
            .map(|active| (active.attempt, Arc::clone(&active.snapshot_cell)))
    }

    /// Starts a new call attempt on `channel`, returning its `(attempt, snapshot_cell)` immediately.
    pub(crate) fn join(
        self: &Arc<Self>,
        channel: Uuid,
        media_factory: Option<Arc<dyn MediaEngineFactory>>,
        ice_fetcher: Arc<dyn IceFetcher>,
        runtime: &tokio::runtime::Handle,
        session_root: &CancellationToken,
    ) -> Result<(CallAttemptId, Arc<StateCell<CallSnapshot>>), CallError> {
        if self.closed.load(Ordering::Acquire) != 0 || session_root.is_cancelled() {
            return Err(CallError::Closed);
        }
        let media_factory = media_factory.ok_or(CallError::MediaUnavailable)?;

        let (attempt, generation, wait_drain, attempt_cancel, snapshot_cell, effects) = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !guard.online {
                return Err(CallError::Offline);
            }
            if guard.resources.generation != guard.generation {
                return Err(CallError::PermissionUnknown);
            }
            match voice_access(&guard.resources, self.account.user(), channel) {
                VoiceAccess::Allowed => {}
                VoiceAccess::NotVoiceChannel => return Err(CallError::NotVoiceChannel),
                VoiceAccess::Denied => return Err(CallError::Forbidden),
                VoiceAccess::Unknown => return Err(CallError::PermissionUnknown),
            }

            let prev_drain = guard.active.as_ref().and_then(|active| {
                if active.media_lease.is_some() || active.media_start_in_flight {
                    Some(active.attempt)
                } else {
                    None
                }
            });
            let (attempt, wait_drain) =
                self.client_lease
                    .acquire(self.account, channel, prev_drain)?;

            let mut effects = SideEffects::default();
            if guard.active.is_some() {
                self.finish_active_locked(&mut guard, CallEndReason::Switched, true, &mut effects);
            }
            guard.reducer.clear_attempt_routes();
            guard.reducer.clear_room_speakers(channel);

            let generation = guard.generation;
            let cancel = session_root.child_token();
            let (active, cell) =
                ActiveAttempt::new(self.account, generation, attempt, channel, cancel.clone());
            let snap = active.snapshot(self.account, &guard.reducer);
            active.snapshot_cell.send_replace(snap.clone());
            effects.active_publish = ActivePublish::Set(Box::new(snap));
            guard.active = Some(active);
            (attempt, generation, wait_drain, cancel, cell, effects)
        };

        self.execute_effects(effects, runtime);
        startup::spawn_prepare(
            Arc::clone(self),
            attempt,
            generation,
            wait_drain,
            attempt_cancel,
            media_factory,
            ice_fetcher,
            runtime,
        );
        Ok((attempt, snapshot_cell))
    }

    /// Returns the active peer reference once `voice_joined` has established `peer_id`.
    pub(crate) fn active_peer_ref(active: Option<&ActiveAttempt>) -> Option<ActivePeerRef<'_>> {
        let active = active?;
        if !matches!(active.phase, CallPhase::Joined | CallPhase::Connected) {
            return None;
        }
        let peer_id = active.peer_id.as_deref()?;
        Some(ActivePeerRef {
            channel: active.channel,
            peer_id,
        })
    }

    /// Reconciles pending video subscriptions against current publishers and pacing limits.
    pub(crate) fn reconcile_subscriptions_locked(
        &self,
        guard: &mut CoordinatorState,
        effects: &mut SideEffects,
    ) {
        let Some(active) = guard.active.as_mut() else {
            return;
        };
        if !matches!(active.phase, CallPhase::Joined | CallPhase::Connected) {
            return;
        }
        let now = Instant::now();
        let commands = active.subscriptions.reconcile(
            active.channel,
            self.account.user(),
            &guard.reducer,
            now,
        );
        for command in commands {
            effects.wire_commands.push(QueuedVoiceCommand {
                generation: active.generation,
                attempt: active.attempt,
                command,
                allow_after_end: false,
            });
        }
        if let Some(at) =
            active
                .subscriptions
                .next_wakeup(active.channel, self.account.user(), &guard.reducer)
        {
            effects.schedule_retry_at = Some((active.attempt, at));
        }
    }

    /// Ends the currently active attempt under the coordinator lock and stages cleanup effects.
    pub(crate) fn finish_active_locked(
        &self,
        guard: &mut CoordinatorState,
        reason: CallEndReason,
        send_leave: bool,
        effects: &mut SideEffects,
    ) {
        let Some(mut active) = guard.active.take() else {
            return;
        };
        active.cancel.cancel();
        if send_leave && active.voice_join_sent {
            if active.phase == CallPhase::Joining {
                *guard.abandoned_joins.entry(active.channel).or_default() += 1;
            }
            effects.wire_commands.push(QueuedVoiceCommand {
                generation: active.generation,
                attempt: active.attempt,
                command: Command::VoiceLeave {
                    channel_id: active.channel.to_string(),
                },
                allow_after_end: true,
            });
        }
        let needs_drain = active.media_lease.is_some() || active.media_start_in_flight;
        if let Some(lease) = active.media_lease.take() {
            guard.draining_attempts.insert(active.attempt);
            effects.stop_leases.push((active.attempt, lease));
        } else if active.media_start_in_flight {
            guard.draining_attempts.insert(active.attempt);
        }
        self.client_lease.begin_end(active.attempt, needs_drain);
        for destination in active.destinations.into_values() {
            effects.revoke_destinations.push(destination);
        }
        active.destinations = BTreeMap::new();
        active.phase = CallPhase::Ended(reason);
        active.version = active.version.saturating_add(1);
        guard.reducer.clear_attempt_routes();
        guard.reducer.clear_room_speakers(active.channel);
        let final_snapshot = active.snapshot(self.account, &guard.reducer);
        effects
            .closed_cells
            .push((Arc::clone(&active.snapshot_cell), final_snapshot));
        effects.active_publish = ActivePublish::Clear;
    }

    /// Stages a fresh `RoomsSnapshot` publication outside the coordinator lock.
    pub(crate) fn stage_rooms_publish(
        &self,
        guard: &mut CoordinatorState,
        effects: &mut SideEffects,
    ) {
        guard.rooms_version = guard.rooms_version.saturating_add(1);
        let snapshot = guard.reducer.project_rooms(
            guard.rooms_version,
            guard.online,
            &guard.resources,
            self.account.user(),
        );
        effects.rooms_publish = Some(snapshot);
    }

    /// Stages an active `CallSnapshot` publication outside the coordinator lock.
    pub(crate) fn stage_active_publish(
        &self,
        guard: &mut CoordinatorState,
        effects: &mut SideEffects,
    ) {
        let Some(active) = guard.active.as_mut() else {
            return;
        };
        active.version = active.version.saturating_add(1);
        let snap = active.snapshot(self.account, &guard.reducer);
        active.snapshot_cell.send_replace(snap.clone());
        effects.active_publish = ActivePublish::Set(Box::new(snap));
    }

    /// Executes deferred side effects outside the coordinator lock with a live Tokio runtime.
    pub(crate) fn execute_effects(
        self: &Arc<Self>,
        effects: SideEffects,
        runtime: &tokio::runtime::Handle,
    ) {
        self.execute_effects_optional(effects, Some(runtime));
    }

    /// Executes deferred side effects outside the coordinator lock.
    pub(crate) fn execute_effects_optional(
        self: &Arc<Self>,
        effects: SideEffects,
        runtime: Option<&tokio::runtime::Handle>,
    ) {
        for destination in effects.revoke_destinations {
            destination.revoke();
        }
        // Enqueue wire commands BEFORE dispatching media commands so camera intent always
        // enters the ordered voice FIFO ahead of any offer emitted by the media engine.
        let mut overflowed = false;
        if !effects.wire_commands.is_empty() {
            let sender = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .command_sender
                .clone();
            if let Some((sender_gen, sender)) = sender {
                for item in effects.wire_commands {
                    if item.generation != sender_gen {
                        continue;
                    }
                    if matches!(sender.try_send(item), Err(EnqueueError::Full)) {
                        overflowed = true;
                        break;
                    }
                }
            }
        }
        // Publish state transitions and closed cells before telling old leases to stop so
        // callers observing a stopped lease already see the ended call snapshot.
        for (cell, final_snapshot) in effects.closed_cells {
            cell.close_with(|slot| {
                *slot = final_snapshot;
                true
            });
        }
        match effects.active_publish {
            ActivePublish::Unchanged => {}
            ActivePublish::Clear => {
                self.active_cell.send_replace(None);
            }
            ActivePublish::Set(snap) => {
                self.active_cell.send_replace(Some(*snap));
            }
        }
        if let Some(rooms) = effects.rooms_publish {
            self.rooms_cell.send_replace(rooms);
        }
        for (attempt, lease) in effects.stop_leases {
            let _ = lease.send(MediaCommand::Stop);
            if let Some(rt) = runtime {
                startup::spawn_drain_watchdog(
                    Arc::clone(&self.client_lease),
                    attempt,
                    self.config.drain_timeout,
                    rt,
                );
            }
        }
        if !effects.media_commands.is_empty() {
            let guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(lease) = guard.active.as_ref().and_then(|a| a.media_lease.as_ref()) {
                for command in effects.media_commands {
                    let _ = lease.send(command);
                }
            }
        }
        if overflowed {
            let _ = self.end_active(CallEndReason::CommandOverflow, runtime);
        } else if let Some((attempt, at)) = effects.schedule_retry_at
            && let Some(rt) = runtime
        {
            startup::spawn_subscription_wakeup(Arc::clone(self), attempt, at, rt);
        }
    }
}
