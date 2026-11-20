//! Session-wide lifecycle reactions for `SessionCalls`: resource permission re-evaluation,
//! connection status transitions, logout/removal/shutdown drain and wakeup timers.

use crate::{
    ConnectionState, Generation, ResourceSnapshot,
    permissions::{VoiceAccess, voice_access},
};
use std::sync::{Arc, atomic::Ordering};

use super::{
    attempt::SideEffects,
    coordinator::SessionCalls,
    model::{CallAttemptId, CallEndReason, CallPhase},
};

impl SessionCalls {
    /// Re-evaluates room visibility and active-call permission after a resource snapshot commits.
    pub(crate) fn on_resources_updated(
        self: &Arc<Self>,
        snapshot: &Arc<ResourceSnapshot>,
        runtime: Option<&tokio::runtime::Handle>,
    ) {
        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if snapshot.generation != guard.generation {
                return;
            }
            guard.resources = Arc::clone(snapshot);
            if let Some(active) = guard.active.as_ref() {
                match voice_access(snapshot, self.account.user(), active.channel) {
                    VoiceAccess::Allowed => {}
                    VoiceAccess::Denied | VoiceAccess::NotVoiceChannel => {
                        let reason = if snapshot.channels.get(active.channel).is_none() {
                            CallEndReason::RoomClosed
                        } else {
                            CallEndReason::Forbidden
                        };
                        self.finish_active_locked(&mut guard, reason, true, &mut effects);
                    }
                    VoiceAccess::Unknown => {
                        self.finish_active_locked(
                            &mut guard,
                            CallEndReason::PermissionUnknown,
                            true,
                            &mut effects,
                        );
                    }
                }
            }
            self.stage_rooms_publish(&mut guard, &mut effects);
        }
        self.execute_effects_optional(effects, runtime);
    }

    /// Updates connection status; leaving `Online` clears live rooms and ends any active call.
    pub(crate) fn on_connection_status(
        self: &Arc<Self>,
        generation: Generation,
        state: ConnectionState,
        resources: &Arc<ResourceSnapshot>,
        runtime: Option<&tokio::runtime::Handle>,
    ) {
        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.generation != generation {
                if guard.active.is_some() {
                    self.finish_active_locked(
                        &mut guard,
                        CallEndReason::ConnectionLost,
                        false,
                        &mut effects,
                    );
                }
                guard.generation = generation;
                guard.reducer.reset_generation(generation);
                guard.abandoned_joins.clear();
                guard.command_sender = None;
            }
            guard.resources = Arc::clone(resources);
            guard.online = state == ConnectionState::Online;
            if !guard.online {
                if guard.active.is_some() {
                    self.finish_active_locked(
                        &mut guard,
                        CallEndReason::ConnectionLost,
                        false,
                        &mut effects,
                    );
                }
                guard.reducer.reset_generation(generation);
                guard.abandoned_joins.clear();
            }
            self.stage_rooms_publish(&mut guard, &mut effects);
        }
        self.execute_effects_optional(effects, runtime);
    }

    /// Ends any active call for logout, account removal or shutdown; returns any draining attempt.
    pub(crate) fn end_active(
        self: &Arc<Self>,
        reason: CallEndReason,
        runtime: Option<&tokio::runtime::Handle>,
    ) -> Option<CallAttemptId> {
        let mut effects = SideEffects::default();
        let draining = {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let send_leave = matches!(
                reason,
                CallEndReason::LocalLeave
                    | CallEndReason::Switched
                    | CallEndReason::HostTerminated
                    | CallEndReason::LoggedOut
                    | CallEndReason::AccountRemoved
                    | CallEndReason::Shutdown
            );
            let active_attempt = guard.active.as_ref().and_then(|active| {
                if active.media_lease.is_some() || active.media_start_in_flight {
                    Some(active.attempt)
                } else {
                    None
                }
            });
            if guard.active.is_some() {
                self.finish_active_locked(&mut guard, reason, send_leave, &mut effects);
            }
            active_attempt.or_else(|| guard.draining_attempts.iter().next().copied())
        };
        self.execute_effects_optional(effects, runtime);
        draining
    }

    /// Stops any active call, awaits media drain within `config.drain_timeout`, and closes cells.
    pub(crate) async fn stop_and_drain(self: &Arc<Self>, reason: CallEndReason) {
        self.closed.store(1, Ordering::Release);
        let draining = self.end_active(reason, None);
        if let Some(attempt) = draining {
            let timeout = self.config.drain_timeout;
            if tokio::time::timeout(timeout, self.client_lease.wait_drained(attempt))
                .await
                .is_err()
            {
                self.client_lease.mark_release_pending(attempt);
            }
        }
        self.rooms_cell.close();
        self.active_cell.close();
    }

    /// Invoked by `startup::spawn_subscription_wakeup` when a backoff or rate-limit timer fires.
    pub(crate) fn on_subscription_wakeup(
        self: &Arc<Self>,
        attempt: CallAttemptId,
        runtime: &tokio::runtime::Handle,
    ) {
        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard
                .active
                .as_ref()
                .is_some_and(|active| active.attempt == attempt && active.phase.is_active())
            {
                self.reconcile_subscriptions_locked(&mut guard, &mut effects);
            }
        }
        self.execute_effects(effects, runtime);
    }

    /// Invoked by `startup` when `join_timeout` expires before `voice_joined` arrives.
    pub(crate) fn on_join_timeout(
        self: &Arc<Self>,
        attempt: CallAttemptId,
        runtime: &tokio::runtime::Handle,
    ) {
        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.active.as_ref().is_some_and(|active| {
                active.attempt == attempt && active.phase == CallPhase::Joining
            }) {
                self.finish_active_locked(
                    &mut guard,
                    CallEndReason::JoinTimeout,
                    true,
                    &mut effects,
                );
            }
        }
        self.execute_effects(effects, runtime);
    }
}
